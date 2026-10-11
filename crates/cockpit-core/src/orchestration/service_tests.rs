use super::*;
use super::claims::AttachmentReference;
use std::path::PathBuf;

const SESSION: &str = "isolated-service-test";

struct Fixture {
    root: PathBuf,
    project: ProjectIdentity,
    configuration: ProjectConfiguration,
    service: OrchestrationService,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("cockpit-orchestration-service-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (mut project, _) = project_identity::resolve_owned_directory(&root).unwrap();
        project.source_workspace_id = Some("test-space".into());
        let configuration = ProjectConfiguration {
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
        let service = OrchestrationService::open(&configuration).unwrap();
        Self {
            root,
            project,
            configuration,
            service,
        }
    }

    fn state(&self) -> OrchestrationState {
        self.service.store.lock().unwrap().read().unwrap()
    }

    fn run(&self, run_id: &str) -> Run {
        self.state()
            .runs
            .into_iter()
            .find(|run| run.run_id == run_id)
            .unwrap()
    }

    fn task(&self, root_id: &str, task_id: &str) -> Task {
        let locked = self.service.store.lock().unwrap();
        locked
            .tasks(root_id)
            .unwrap()
            .task(task_id)
            .unwrap()
            .clone()
    }

    fn apply(
        &self,
        actor: &Actor,
        action: OrchestrationAction,
    ) -> Result<OrchestrationMutationResponse, InspectionError> {
        let expected_revision = matches!(actor, Actor::Operator(_)).then(|| self.state().revision);
        self.service.mutate_resolved(
            actor,
            OrchestrationMutationRequest {
                session_id: SESSION.into(),
                expected_revision,
                action,
            },
            &ResolvedMutationInput {
                project: Some(self.project()),
                ..ResolvedMutationInput::default()
            },
        )
    }

    fn operator(&self, action: OrchestrationAction) -> OrchestrationMutationResponse {
        self.apply(&Actor::Operator(OperatorOrigin::Browser), action)
            .unwrap()
    }

    fn project(&self) -> ProjectIdentity {
        self.project.clone()
    }

    fn root(&self) -> String {
        let response = self.operator(OrchestrationAction::SupervisorStart {
            operation_id: id(),
            mode: SupervisorStartMode::Another,
            target: Some(target()),
            label: Some("Test supervisor".into()),
            selected_root_id: None,
            expected_project_key: Some(self.project().key),
        });
        match response.result {
            OrchestrationActionResult::SupervisorStarted { start } => start.root_id,
            other => panic!("Expected supervisor start, got {other:?}"),
        }
    }

    fn auth(&self, run_id: &str, role: JobRole, child: Option<&str>) -> JobAuth {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let index = run_index(&state, run_id).unwrap();
        if state.runs[index].kind == RunKind::Worker
            && state.runs[index].checkout_claim_id.is_none()
        {
            let attachment = state.runs[index].attachment.as_ref().unwrap();
            let holder = AttachmentReference {
                run_id: run_id.into(),
                attachment_id: attachment.attachment_id.clone(),
                attachment_epoch: attachment.epoch,
            };
            let path = self.root.join("checkouts").join(run_id);
            std::fs::create_dir_all(&path).unwrap();
            let (_, checkout) = project_identity::resolve_owned_directory(&path).unwrap();
            claims::reserve_claim(&mut state, &checkout, &holder, &id()).unwrap();
        }
        let (credential, path) =
            credentials::stage_credential(self.service.base(), &state.runs[index], role, child).unwrap();
        state.credentials.push(credential);
        locked.save(&mut state).unwrap();
        credentials::read_job_auth(&path).unwrap()
    }

    fn child_actor(&self, parent: &Actor, run_id: &str, child_id: &str) -> Actor {
        let mut child = parent.clone();
        let caller = caller_mut(&mut child);
        caller.auth = self.auth(run_id, JobRole::Child, Some(child_id));
        caller.agent_kind = Some(AgentKind::Subagent);
        caller.subagent_id = Some(child_id.into());
        caller.omp_session_id = Some(format!("{child_id}-native"));
        caller.provenance = Some(CallerProvenance {
            agent_kind: caller.agent_kind,
            child_id: caller.subagent_id.clone(),
            main_session_id: caller.main_omp_session_id.clone(),
            native_session_id: caller.omp_session_id.clone(),
            process: caller.process.clone(),
        });
        child
    }

    fn attach_observed(&self, actor: &Actor, run_id: &str) {
        let Actor::Agent(caller) = actor else { unreachable!() };
        let mut attachment = self.run(run_id).attachment.unwrap();
        let location = attachment.location.as_mut().unwrap();
        location.workspace_id = caller.workspace_id.clone();
        location.tab_id = caller.tab_id.clone();
        location.pane_id = caller.pane_id.clone();
        location.native_session_id = caller.native_session_id.clone();
        attachment.process = caller.process.clone();
        attachment.state = AttachmentState::Connected;
        self.service.mutate_resolved(actor, OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: None,
            action: OrchestrationAction::RunAttach {
                operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
                expected_attachment_epoch: caller.auth.attachment_epoch,
                main_session_id: caller.omp_session_id.clone().unwrap(),
            },
        }, &ResolvedMutationInput {
            attachment_observation: Some(attachment), ..ResolvedMutationInput::default()
        }).unwrap();
    }

    fn resume_observed(&self, run_id: &str, observed: ExecutionAttachment) -> RunResumeResult {
        let response = self.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            OrchestrationMutationRequest {
                session_id: SESSION.into(), expected_revision: Some(self.state().revision),
                action: OrchestrationAction::RunResume {
                    run_id: run_id.into(), operation_id: id(), expected_attachment_epoch: Some(observed.epoch),
                    mode: ResumeMode::ReattachOrRestart, decision_id: None,
                },
            }, &ResolvedMutationInput {
                attachment_observation: Some(observed), ..ResolvedMutationInput::default()
            }).unwrap();
        let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
        resume
    }

    fn resume_action(&self, run_id: &str) -> OrchestrationAction {
        OrchestrationAction::RunResume {
            run_id: run_id.into(), operation_id: id(),
            expected_attachment_epoch: self.run(run_id).attachment.as_ref().map(|attachment| attachment.epoch),
            mode: ResumeMode::ReattachOrRestart, decision_id: None,
        }
    }

    fn resume_exited(&self, actor: &Actor, run_id: &str) -> OrchestrationMutationResponse {
        let run = self.run(run_id);
        let attachment = run.attachment.as_ref().unwrap();
        let claim = self.state().checkout_claims.into_iter()
            .find(|claim| run.checkout_claim_id.as_ref() == Some(&claim.claim_id));
        let (claim_revision, checkout) = match claim {
            Some(claim) => (claim.revision, Some(claim.checkout)),
            None => {
                assert_ne!(run.kind, RunKind::Worker);
                (0, None)
            }
        };
        let proof = claims::WriterExitProof {
            incarnation: AttachmentReference {
                run_id: run_id.into(), attachment_id: attachment.attachment_id.clone(),
                attachment_epoch: attachment.epoch,
            },
            claim_revision, checkout,
            machine_id: attachment.machine_id.clone().unwrap(),
            previous_kernel_boot_id: attachment.process.as_ref()
                .or_else(|| attachment.shell.as_ref().map(|shell| &shell.process))
                .unwrap().kernel_boot_id.clone().unwrap(),
            observed_kernel_boot_id: "kernel-two".into(), all_consumers_absent: true,
        };
        self.service.mutate_resolved(actor, OrchestrationMutationRequest {
            session_id: SESSION.into(),
            expected_revision: matches!(actor, Actor::Operator(_)).then(|| self.state().revision),
            action: OrchestrationAction::RunResume {
                run_id: run_id.into(), operation_id: id(), expected_attachment_epoch: Some(attachment.epoch),
                mode: ResumeMode::ReattachOrRestart, decision_id: None,
            },
        }, &ResolvedMutationInput { writer_exit_proof: Some(proof), ..ResolvedMutationInput::default() }).unwrap()
    }
    fn create_task(&self, root_id: &str, title: &str) -> Task {
        if self.run(root_id).stage == RunStage::Starting {
            self.launch_and_bind(root_id);
        }
        task_result(self.operator(OrchestrationAction::TaskCreate {
            root_id: root_id.into(),
            title: title.into(),
            task_id: id(),
            description: "Canonical task body".into(),
            depends_on: Vec::new(),
            follow_up_of: None,
            expected_doc_revision: None,
            source_revision: None,
        }))
    }

    fn start(&self, parent: &str, task: &Task) -> String {
        let root = self.run(parent).root_id;
        let run_id = run_result(self.operator(self.start_action(&root, task))).0;
        if parent != root {
            // Persisted ancestry remains meaningful for retention and delivery;
            // current WorkerStart always creates a direct child of the root.
            self.seed_run(&run_id, |run| run.parent_run_id = Some(parent.into()));
        }
        run_id
    }

    fn start_action(&self, root: &str, task: &Task) -> OrchestrationAction {
        OrchestrationAction::WorkerStart {
            operation_id: id(),
            root_id: root.into(),
            task_id: task.task_id.clone(),
            expected_task_revision: task.task_revision.clone(),
            expected_doc_revision: task_document_revision(self, root),
            target: target(),
            label: None,
            brief: "Implement the canonical task and report evidence".into(),
            cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe,
        }
    }

    fn setup(&self, run_id: &str) {
        self.service.record_dispatch(run_id, DispatchUpdate::SetupPlanned {
            setup: SetupSummary {
                operation_id: None, generation: None, workspace_id: Some("test-space".into()),
                checkout_path: if self.run(run_id).kind == RunKind::Worker {
                    self.root.join("checkouts").join(run_id).to_string_lossy().into_owned()
                } else { self.root.to_string_lossy().into_owned() },
                repository_id: None, branch: None, base: None, ownership: None,
                base_commit: None,
                effects: vec!["Open isolated existing Space".into()], warnings: Vec::new(),
                project_workspace_id: None,
            },
        }).unwrap();
    }
    fn launch_and_bind(&self, run_id: &str) -> Actor {
        let actor = self.launch_tab(run_id);
        self.service
            .record_launch_pending(&self.run(run_id))
            .unwrap();
        let Actor::Agent(caller) = &actor else {
            unreachable!();
        };
        self.apply(
            &actor,
            OrchestrationAction::RunAttach { operation_id: id(), attachment_id: caller.auth.attachment_id.clone(), expected_attachment_epoch: caller.auth.attachment_epoch, main_session_id: caller.omp_session_id.clone().unwrap() },
        )
        .unwrap();
        self.service
            .record_launch_verified(&self.run(run_id))
            .unwrap();
        actor
    }

    // Persist real launch intent/tab receipts, but leave the asynchronous first
    // start receipt outstanding so same-launch lifecycle races can be exercised.
    fn launch_tab(&self, run_id: &str) -> Actor {
        let run = self.run(run_id);
        let attempt = run.dispatch.as_ref().unwrap().launch_attempt.max(1);
        let native = format!("native-{run_id}-{attempt}");
        let location = RunLocation {
            boot_id: Some("boot-one".into()),
            terminal_id: Some(format!("terminal-{run_id}-{attempt}")),
            native_session_id: Some(native.clone()),
            endpoint_identity: "endpoint-one".into(),
            session_id: SESSION.into(),
            workspace_id: "test-space".into(),
            tab_id: format!("tab-{run_id}-{attempt}"),
            pane_id: format!("pane-{run_id}-{attempt}"),
            launch_tag: format!("launch-{run_id}-{attempt}"),
        };
        self.service
            .record_dispatch(
                run_id,
                DispatchUpdate::LaunchIntent {
                    launch_tag: location.launch_tag.clone(),
                    launch_attempt: attempt,
                    endpoint_identity: location.endpoint_identity.clone(),
                },
            )
            .unwrap();
        self.service
            .record_dispatch(
                run_id,
                DispatchUpdate::TabReceipt {
                    location: location.clone(),
                    launch_shell_identity: Some(NativeShellIdentity {
                        process: NativeProcessIdentity { pid: 11, start_ticks: 12, kernel_boot_id: Some("kernel-one".into()) },
                        executable_device: "1".into(), executable_inode: "2".into(), argv_digest: "a".repeat(64),
                    }),
                },
            )
            .unwrap();
        self.seed_run(run_id, |run| run.attachment.as_mut().unwrap().machine_id = Some(self.project.machine_id.clone()));
        Actor::Agent(AgentCaller {
            auth: self.auth(run_id, if run.kind == RunKind::Worker { JobRole::Worker } else { JobRole::Root }, None),
            provenance: None,
            endpoint_identity: location.endpoint_identity,
            session_id: SESSION.into(),
            workspace_id: location.workspace_id,
            tab_id: location.tab_id,
            pane_id: location.pane_id,
            boot_id: location.boot_id,
            terminal_id: location.terminal_id,
            native_session_id: location.native_session_id,
            env_run: Some((run_id.into(), run.attempt)),
            omp_session_id: Some(native.clone()),
            main_omp_session_id: Some(native.clone()),
            agent_kind: Some(AgentKind::Main),
            actual_agent_kind: Some(NativeAgentKind::Omp),
            subagent_id: None,
            process: Some(NativeProcessIdentity {
                pid: 42,
                start_ticks: 123,
                kernel_boot_id: Some("kernel-one".into()),
            }),
        })
    }


    fn successful_result(&self, actor: &Actor) -> OrchestrationAction {
        let Actor::Agent(caller) = actor else { panic!("Result requires a job actor") };
        let credential = self.state().credentials.into_iter()
            .find(|credential| credential.credential_id == caller.auth.credential_id).unwrap();
        let run = self.run(&credential.run_id);
        let task_revision = run.task_id.as_ref()
            .map(|task_id| self.task(&run.root_id, task_id).task_revision)
            .unwrap_or_else(|| "coordinator-output".into());
        assert!(run.checkout.as_ref().is_none_or(|checkout| checkout.git.is_none()),
            "Synthetic consumer fixtures must not manufacture code provenance");
        let mut action = report(ReportKind::Result, Some(ReportOutcome::Succeeded), None);
        if let OrchestrationAction::Report { message_id, source_output, .. } = &mut action {
            *source_output = Some(SourceOutput::NonCode {
                result_message_id: message_id.clone(), task_revision,
                artifacts: Vec::new(), focused_checks: Vec::new(),
            });
        }
        action
    }
    fn initialized(&self, actor: &Actor) -> OrchestrationMutationResponse {
        self.apply(actor, report(ReportKind::Initialized, None,
            Some("Implement the task, then report evidence"))).unwrap()
    }

    fn working(&self, parent_run_id: &str) -> (Task, String, Actor) {
        let root_id = self.run(parent_run_id).root_id;
        let task = self.create_task(&root_id, "Worker task");
        let run_id = self.start(parent_run_id, &task);
        self.setup(&run_id);
        let actor = self.launch_and_bind(&run_id);
        self.initialized(&actor);
        (task, run_id, actor)
    }

    // Seed the exact durable state left by a crash after writing the acceptance
    // intent, before checking tasks.md or finalizing the machine record.
    fn pending_intent(&self, root_id: &str, task: &Task, run_id: &str) -> String {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let intent_id = id();
        let result_message_id = state.runs[run_index(&state, run_id).unwrap()]
            .result.as_ref().expect("reviewed successful Result").message_id.clone();
        state.task_intents.push(TaskIntent {
            intent_id: intent_id.clone(),
            root_id: root_id.into(),
            task_id: task.task_id.clone(),
            run_id: run_id.into(),
            expected_task_revision: task.task_revision.clone(),
            state: IntentState::Pending,
            origin: Some(DecisionOrigin::Browser),
            supervisor_run_id: None,
            omp_session_id: None,
            result_message_id: Some(result_message_id),
        });
        locked.save(&mut state).unwrap();
        intent_id
    }

    fn state_bytes(&self) -> Vec<u8> {
        std::fs::read(self.service.base().join("state.json")).unwrap()
    }

    fn seed_run(&self, run_id: &str, change: impl FnOnce(&mut Run)) {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let index = run_index(&state, run_id).unwrap();
        change(&mut state.runs[index]);
        locked.save(&mut state).unwrap();
    }

    fn seed_state(&self, change: impl FnOnce(&mut OrchestrationState)) {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        change(&mut state);
        locked.save(&mut state).unwrap();
    }

    

    
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn target() -> DispatchTarget {
    DispatchTarget::ExistingSpace {
        workspace_id: "test-space".into(),
    }
}


fn report(
    kind: ReportKind,
    outcome: Option<ReportOutcome>,
    plan: Option<&str>,
) -> OrchestrationAction {
    OrchestrationAction::Report {
        message_id: id(),
        kind,
        outcome,
        summary: "Explicit worker evidence".into(),
        plan: plan.map(str::to_owned),
        source_output: None,
        to_run_id: None,
        original_task_revision: None,
        original_claim_id: None,
    }
}

fn run_result(response: OrchestrationMutationResponse) -> (String, u32) {
    match response.result {
        OrchestrationActionResult::Run { run_id, attempt } => (run_id, attempt),
        OrchestrationActionResult::WorkerStarted { start } => (start.run_id, start.attempt),
        OrchestrationActionResult::SupervisorStarted { start } => (start.root_id, 1),
        other => panic!("Expected a run result, got {other:?}"),
    }
}

fn task_result(response: OrchestrationMutationResponse) -> Task {
    match response.result {
        OrchestrationActionResult::Task { task } => task,
        other => panic!("Expected a task result, got {other:?}"),
    }
}

fn caller_mut(actor: &mut Actor) -> &mut AgentCaller {
    match actor {
        Actor::Agent(caller) => caller,
        _ => panic!("Expected an agent actor"),
    }
}

fn live_process() -> NativeProcessIdentity {
    let pid = std::process::id();
    NativeProcessIdentity {
        pid,
        start_ticks: crate::process_identity::start_identity(i32::try_from(pid).unwrap()).unwrap(),
        kernel_boot_id: crate::process_identity::kernel_boot_id(),
    }
}

fn assert_code(result: Result<OrchestrationMutationResponse, InspectionError>, code: &str) {
    assert_eq!(result.unwrap_err().code, code);
}

#[test]
fn machine_revision_cas_rejects_stale_service_and_preserves_durable_state() {
    let fixture = Fixture::new();
    let other = OrchestrationService::open(&fixture.configuration).unwrap();
    let initial = other.store.lock().unwrap().read().unwrap().revision;
    let root = fixture.root();
    let committed = fixture.state().revision;
    assert!(committed > initial);
    assert_code(
        other.mutate(
            &Actor::Operator(OperatorOrigin::Native),
            OrchestrationMutationRequest {
                session_id: SESSION.into(),
                expected_revision: Some(initial),
                action: OrchestrationAction::CancelRun {
                    run_id: root.clone(),
                },
            },
        ),
        "orchestration_revision_conflict",
    );
    assert_eq!(fixture.state().revision, committed);
    assert_eq!(fixture.run(&root).stage, RunStage::Starting);
    let response = other
        .mutate(
            &Actor::Operator(OperatorOrigin::Native),
            OrchestrationMutationRequest {
                session_id: SESSION.into(),
                expected_revision: Some(committed),
                action: OrchestrationAction::CancelRun {
                    run_id: root.clone(),
                },
            },
        )
        .unwrap();
    assert_eq!(response.revision, committed + 1);
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    let state = reopened.store.lock().unwrap().read().unwrap();
    assert_eq!(state.revision, response.revision);
    assert_eq!(
        state.runs[run_index(&state, &root).unwrap()].close_reason,
        Some(CloseReason::Cancelled)
    );
}

#[test]
fn canonical_task_mutations_are_authorized_only_in_the_agents_own_root() {
    let fixture = Fixture::new();
    let own_root = fixture.root();
    let other_root = fixture.root();
    let actor = fixture.launch_and_bind(&own_root);
    let before = fixture.state().revision;
    let own_task = task_result(
        fixture
            .apply(
                &actor,
                OrchestrationAction::TaskCreate {
                    root_id: own_root.clone(),
                    title: "Own task".into(),
                    task_id: id(),
                    description: "Own body".into(),
                    depends_on: Vec::new(), follow_up_of: None,
                    expected_doc_revision: None, source_revision: None,
                },
            )
            .unwrap(),
    );
    assert_eq!(
        fixture.state().revision,
        before,
        "Task content is not machine state"
    );
    let other_task = fixture.create_task(&other_root, "Other task");
    let own_doc_revision = fixture
        .service
        .store
        .lock()
        .unwrap()
        .tasks(&own_root)
        .unwrap()
        .doc_revision
        .clone();
    fixture
        .apply(
            &actor,
            OrchestrationAction::TasksAssignIds {
                root_id: own_root.clone(),
                expected_doc_revision: own_doc_revision,
            },
        )
        .unwrap();
    let updated = task_result(
        fixture
            .apply(
                &actor,
                OrchestrationAction::TaskUpdate {
                    root_id: own_root.clone(),
                    task_id: own_task.task_id.clone(),
                    expected_task_revision: own_task.task_revision,
                    title: Some("Updated own task".into()),
                    description: None,
                },
            )
            .unwrap(),
    );
    assert_eq!(updated.title, "Updated own task");
    let other_bytes = std::fs::read(
        fixture
            .service
            .base()
            .join("tasks")
            .join(format!("{other_root}.md")),
    )
    .unwrap();
    let other_doc_revision = fixture
        .service
        .store
        .lock()
        .unwrap()
        .tasks(&other_root)
        .unwrap()
        .doc_revision
        .clone();
    for action in [
        OrchestrationAction::TaskCreate {
            root_id: other_root.clone(),
            title: "Intrusion".into(),
            task_id: id(),
            description: String::new(),
            depends_on: Vec::new(), follow_up_of: None,
            expected_doc_revision: None, source_revision: None,
        },
        OrchestrationAction::TaskUpdate {
            root_id: other_root.clone(),
            task_id: other_task.task_id.clone(),
            expected_task_revision: other_task.task_revision.clone(),
            title: Some("Intrusion".into()),
            description: None,
        },
        OrchestrationAction::TasksAssignIds {
            root_id: other_root.clone(),
            expected_doc_revision: other_doc_revision,
        },
    ] {
        assert_code(fixture.apply(&actor, action), "actor_forbidden");
    }
    assert_eq!(
        std::fs::read(
            fixture
                .service
                .base()
                .join("tasks")
                .join(format!("{other_root}.md"))
        )
        .unwrap(),
        other_bytes
    );
    assert_eq!(
        fixture.task(&other_root, &other_task.task_id).title,
        other_task.title
    );
}

#[test]
fn worker_start_replays_without_new_writer_and_fences_changed_payload() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Single start");
    let action = fixture.start_action(&root, &task);
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::WorkerStarted { start } = first.result else { unreachable!() };
    assert!(!start.duplicate);
    let run = fixture.run(&start.run_id);
    assert_eq!(run.task_revision_at_start.as_deref(), Some(task.task_revision.as_str()));
    assert_eq!(run.cleanup_policy, Some(WorkerCleanupPolicy::OwnedWhenSafe));
    assert_eq!(run.attachment.as_ref().unwrap().epoch, 1);
    let bytes = fixture.state_bytes();
    let replay = fixture.operator(action.clone());
    let OrchestrationActionResult::WorkerStarted { start: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.run_id, start.run_id);
    assert_eq!(fixture.state_bytes(), bytes);
    let mut changed = action;
    let OrchestrationAction::WorkerStart { brief, .. } = &mut changed else { unreachable!() };
    *brief = "A different operation payload".into();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        fixture.start_action(&root, &task)), "task_has_active_run");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn attachment_not_initialized_report_starts_work_and_plans_are_advisory() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "One start");
    let run_id = fixture.start(&root, &task);
    fixture.setup(&run_id);
    let actor = fixture.launch_and_bind(&run_id);
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    fixture.initialized(&actor);
    let run = fixture.run(&run_id);
    assert_eq!(run.stage, RunStage::Working);
    assert_eq!(run.last_report.as_ref().unwrap().kind, ReportKind::Initialized);
    assert_eq!(run.last_report.as_ref().unwrap().plan.as_deref(),
        Some("Implement the task, then report evidence"));
    fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
    assert!(!fixture.task(&root, &task.task_id).checked);
}

#[test]
fn successful_result_does_not_check_task_until_operator_accepts_exact_task_revision() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, actor) = fixture.working(&root);
    fixture.apply(&actor, fixture.successful_result(&actor))
        .unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
    assert!(!fixture.task(&root, &task.task_id).checked);
    assert!(fixture.state().task_intents.is_empty());
    assert_code(
        fixture.apply(
            &actor,
            OrchestrationAction::Accept {
                run_id: run_id.clone(),
                expected_task_revision: task.task_revision.clone(),
            },
        ),
        "actor_forbidden",
    );
    let edited = task_result(fixture.operator(OrchestrationAction::TaskUpdate {
        root_id: root.clone(),
        task_id: task.task_id.clone(),
        expected_task_revision: task.task_revision.clone(),
        title: Some("Operator refined acceptance criteria".into()),
        description: None,
    }));
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::Accept {
                run_id: run_id.clone(),
                expected_task_revision: task.task_revision,
            },
        ),
        "task_revision_conflict",
    );
    assert!(!fixture.task(&root, &task.task_id).checked);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationAction::Accept {
            run_id: run_id.clone(), expected_task_revision: edited.task_revision.clone(),
        }), "source_output_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.operator(OrchestrationAction::SendBack {
        run_id: run_id.clone(), text: "Review the updated canonical criteria".into(),
    });
    fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
    assert_ne!(edited.task_revision, fixture.run(&run_id).task_revision_at_start.unwrap());
    fixture.operator(OrchestrationAction::Accept {
        run_id: run_id.clone(),
        expected_task_revision: edited.task_revision,
    });
    assert!(fixture.task(&root, &task.task_id).checked);
    let accepted = fixture.run(&run_id);
    assert_eq!(accepted.stage, RunStage::Closed);
    assert_eq!(accepted.close_reason, Some(CloseReason::Accepted));
    assert_eq!(
        accepted.result.unwrap().outcome,
        Some(ReportOutcome::Succeeded)
    );
    assert!(fixture.state().task_intents.is_empty());
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    assert!(
        reopened
            .store
            .lock()
            .unwrap()
            .tasks(&root)
            .unwrap()
            .task(&task.task_id)
            .unwrap()
            .checked
    );
}

#[test]
fn failed_result_is_not_accepted_or_checked() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, actor) = fixture.working(&root);
    fixture
        .apply(
            &actor,
            report(ReportKind::Result, Some(ReportOutcome::Failed), None),
        )
        .unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
    assert_eq!(
        fixture.run(&run_id).result.unwrap().outcome,
        Some(ReportOutcome::Failed)
    );
    let revision = fixture.state().revision;
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::Accept {
                run_id: run_id.clone(),
                expected_task_revision: task.task_revision,
            },
        ),
        "invalid_stage",
    );
    assert_eq!(fixture.state().revision, revision);
    assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
    assert!(fixture.run(&run_id).close_reason.is_none());
    assert!(!fixture.task(&root, &task.task_id).checked);
    assert!(fixture.state().task_intents.is_empty());
}

fn wait_request(fixture: &Fixture, timeout_ms: u32) -> OrchestrationWaitRequest {
    OrchestrationWaitRequest {
        after_revision: fixture.state().revision,
        after_tasks_token: fixture.service.store.tasks_token().unwrap(),
        timeout_ms,
    }
}

// Polling once establishes a deterministic pending-read barrier without letting
// Tokio auto-advance the paused clock. The real wait registers its own wakers.
async fn poll_wait_once<F: Future>(
    mut future: std::pin::Pin<&mut F>,
) -> std::task::Poll<F::Output> {
    std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx))).await
}

fn completed_wait(
    result: std::task::Poll<Result<OrchestrationWaitResponse, InspectionError>>,
) -> OrchestrationWaitResponse {
    match result {
        std::task::Poll::Ready(response) => response.unwrap(),
        std::task::Poll::Pending => panic!("Expected the consumer wait to complete"),
    }
}

#[tokio::test(start_paused = true)]
async fn wait_already_changed_revision_or_tasks_token_returns_at_zero_timeout() {
    let fixture = Fixture::new();
    let before = wait_request(&fixture, 0);
    let root = fixture.root();
    let task = fixture.create_task(&root, "Already available");
    let current = wait_request(&fixture, 0);
    let requests = [
        OrchestrationWaitRequest {
            after_tasks_token: current.after_tasks_token.clone(),
            ..before
        },
        OrchestrationWaitRequest {
            after_revision: current.after_revision,
            after_tasks_token: "older-task-token".into(),
            timeout_ms: 0,
        },
    ];
    let started = tokio::time::Instant::now();
    for request in requests {
        let mut waiting = Box::pin(fixture.service.wait(&request));
        let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
        assert!(response.changed);
        assert_eq!(response.revision, current.after_revision);
        assert_eq!(response.tasks_token, current.after_tasks_token);
    }
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(fixture.task(&root, &task.task_id).title, "Already available");
}

#[tokio::test(start_paused = true)]
async fn wait_zero_timeout_without_change_returns_current_cursor() {
    let fixture = Fixture::new();
    fixture.root();
    let request = wait_request(&fixture, 0);
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(!response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_eq!(response.tasks_token, request.after_tasks_token);
    assert_eq!(tokio::time::Instant::now(), started);
}

#[tokio::test(start_paused = true)]
async fn wait_watched_task_change_wakes_without_advancing_time() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let request = wait_request(&fixture, 3_000);
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());

    let task = fixture.create_task(&root, "Delivered by the watched service");
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(response.changed);
    assert_eq!(response.revision, fixture.state().revision);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(
        fixture.task(&root, &task.task_id).title,
        "Delivered by the watched service"
    );
}

#[tokio::test(start_paused = true)]
async fn wait_external_service_revision_is_visible_within_wait_budget() {
    let fixture = Fixture::new();
    fixture.root();
    let external = OrchestrationService::open(&fixture.configuration).unwrap();
    let request = wait_request(&fixture, 3_000);
    let before = fixture.state_bytes();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());

    let (new_root, attempt) = run_result(
        external
            .mutate_resolved(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationMutationRequest {
                    session_id: SESSION.into(),
                    expected_revision: Some(request.after_revision),
                    action: OrchestrationAction::SupervisorStart {
                        operation_id: id(),
                        mode: SupervisorStartMode::Another,
                        selected_root_id: None,
                        expected_project_key: Some(fixture.project().key),
                        target: Some(target()),
                        label: Some("External service supervisor".into()),
                    },
                },
                &ResolvedMutationInput { project: Some(fixture.project()), ..ResolvedMutationInput::default() },
            )
            .unwrap(),
    );
    let committed = fixture.state_bytes();
    assert_ne!(committed, before, "external machine mutation must persist a revision");
    // This service has no notification from the external writer.
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(1_000)).await;
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(response.changed);
    assert_eq!(response.revision, fixture.state().revision);
    assert!(response.revision > request.after_revision);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
    let delivered = fixture.run(&new_root);
    assert_eq!(delivered.run_id, new_root);
    assert_eq!(delivered.attempt, attempt);
    assert_eq!(delivered.kind, RunKind::Supervisor);
    assert_eq!(delivered.root_id, new_root);
    assert!(delivered.parent_run_id.is_none());
    assert_eq!(delivered.label, "External service supervisor");
    assert_eq!(fixture.state_bytes(), committed, "wait must not write machine state");
}

#[tokio::test(start_paused = true)]
async fn wait_external_markdown_replacement_delivers_task_without_revision_change() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Original task title");
    let request = wait_request(&fixture, 3_000);
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());

    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let replacement = path.with_extension("replacement");
    let document = std::fs::read_to_string(&path).unwrap();
    let updated = document.replace("Original task title", "Externally revised task title");
    assert_ne!(updated, document);
    std::fs::write(&replacement, updated).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    assert_eq!(fixture.state().revision, request.after_revision);
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());

    tokio::time::advance(Duration::from_millis(1_500)).await;
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_ne!(response.tasks_token, request.after_tasks_token);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
    assert_eq!(
        fixture.task(&root, &task.task_id).title,
        "Externally revised task title"
    );
    assert_eq!(fixture.task(&root, &task.task_id).body, task.body);
}

#[tokio::test(start_paused = true)]
async fn wait_short_timeout_returns_unchanged_at_the_requested_deadline() {
    let fixture = Fixture::new();
    fixture.root();
    let request = wait_request(&fixture, 20);
    let started = tokio::time::Instant::now();
    let before = fixture.state_bytes();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(19)).await;
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(1)).await;
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(!response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_eq!(response.tasks_token, request.after_tasks_token);
    assert_eq!(tokio::time::Instant::now() - started, Duration::from_millis(20));
    assert_eq!(fixture.state_bytes(), before);
}

#[tokio::test(start_paused = true)]
async fn wait_oversized_timeout_is_clipped_to_the_protocol_maximum() {
    let fixture = Fixture::new();
    let request = wait_request(&fixture, u32::MAX);
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(29_999)).await;
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(1)).await;
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(!response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_eq!(response.tasks_token, request.after_tasks_token);
    assert_eq!(tokio::time::Instant::now() - started, Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn wait_cancellation_preserves_state_and_next_wait_rediscovers_change() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let original = fixture.create_task(&root, "Existing task");
    let request = wait_request(&fixture, 3_000);
    let started = tokio::time::Instant::now();
    let before = fixture.state_bytes();
    let before_task = fixture.task(&root, &original.task_id);
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    drop(waiting);
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(
        fixture.task(&root, &original.task_id).task_revision,
        before_task.task_revision
    );

    let arrived = fixture.create_task(&root, "Arrived after cancellation");
    let mut next_wait = Box::pin(fixture.service.wait(&request));
    let response = completed_wait(poll_wait_once(next_wait.as_mut()).await);
    assert!(response.changed);
    assert_eq!(response.revision, fixture.state().revision);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(
        fixture.task(&root, &arrived.task_id).title,
        "Arrived after cancellation"
    );
}

#[tokio::test(start_paused = true)]
async fn wait_subscription_edges_do_not_lose_back_to_back_changes() {
    for mutate_before_first_poll in [true, false] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let request = wait_request(&fixture, 3_000);
        let started = tokio::time::Instant::now();
        let mut waiting = Box::pin(fixture.service.wait(&request));
        if !mutate_before_first_poll {
            assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
        }
        // Cover both a commit before subscription and retained notifications
        // after the first unchanged read, before the consumer is polled again.
        let first = fixture.create_task(&root, "First edge arrival");
        let second = fixture.create_task(&root, "Second edge arrival");
        let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
        assert!(response.changed);
        assert_eq!(response.revision, fixture.state().revision);
        assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
        assert_eq!(tokio::time::Instant::now(), started);
        assert_eq!(fixture.task(&root, &first.task_id).title, "First edge arrival");
        assert_eq!(fixture.task(&root, &second.task_id).title, "Second edge arrival");
    }
}

#[test]
fn recovery_without_pending_intents_does_not_notify_subscribers() {
    let fixture = Fixture::new();
    fixture.root();
    let receiver = fixture.service.subscribe();
    let before = fixture.state_bytes();
    for _ in 0..3 {
        fixture.service.recover_intents().unwrap();
        assert!(!receiver.has_changed().unwrap());
        assert_eq!(fixture.state_bytes(), before);
    }
}

#[test]
fn recovery_observes_external_revision_once() {
    let fixture = Fixture::new();
    let external = OrchestrationService::open(&fixture.configuration).unwrap();
    let mut receiver = external.subscribe();
    let initial_revision = *receiver.borrow();
    fixture.root();
    let revision = fixture.state().revision;
    assert!(revision > initial_revision);
    assert!(!receiver.has_changed().unwrap());
    let before = fixture.state_bytes();

    external.recover_intents().unwrap();
    assert!(receiver.has_changed().unwrap());
    assert_eq!(*receiver.borrow_and_update(), revision);
    assert_eq!(fixture.state_bytes(), before);
    external.recover_intents().unwrap();
    assert!(!receiver.has_changed().unwrap());
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn recovery_publishes_assignment_changes_before_later_recovery_error() {
    let fixture = Fixture::new();
    let roots = [fixture.root(), fixture.root()];
    let task_ids = [id(), id()];
    let paths = roots
        .each_ref()
        .map(|root| fixture.service.base().join("tasks").join(format!("{root}.md")));
    for index in 0..2 {
        // A directory in place of Markdown deterministically interrupts the
        // public assignment after its pending journal has been saved.
        std::fs::create_dir(&paths[index]).unwrap();
        assert!(
            fixture
                .apply(
                    &Actor::Operator(OperatorOrigin::Browser),
                    OrchestrationAction::TaskAssign {
                        root_id: roots[index].clone(),
                        task_id: task_ids[index].clone(),
                        title: format!("Interrupted assignment {index}"),
                        description: "Durable recovery notification".into(),
                    },
                )
                .is_err()
        );
    }
    assert_eq!(fixture.state().assignment_intents.len(), 2);
    std::fs::remove_dir(&paths[0]).unwrap();
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    let mut receiver = reopened.subscribe();
    let initial_revision = *receiver.borrow();

    assert!(reopened.recover_intents().is_err());
    let state = fixture.state();
    assert!(state.revision > initial_revision);
    assert!(receiver.has_changed().unwrap());
    assert_eq!(*receiver.borrow_and_update(), state.revision);
    assert_eq!(state.assignment_intents.len(), 1);
    assert_eq!(state.assignment_intents[0].task_id, task_ids[1]);
    assert_eq!(fixture.task(&roots[0], &task_ids[0]).title, "Interrupted assignment 0");
    assert!(state.messages.iter().any(|message| {
        message.message_id == format!("assign-{}", task_ids[0])
    }));

    // The remaining I/O error alone is not a change and must not self-notify.
    let before = fixture.state_bytes();
    assert!(reopened.recover_intents().is_err());
    assert!(!receiver.has_changed().unwrap());
    assert_eq!(fixture.state_bytes(), before);

    std::fs::remove_dir(&paths[1]).unwrap();
    reopened.recover_intents().unwrap();
    let state = fixture.state();
    assert!(receiver.has_changed().unwrap());
    assert_eq!(*receiver.borrow_and_update(), state.revision);
    assert!(state.assignment_intents.is_empty());
    assert_eq!(fixture.task(&roots[1], &task_ids[1]).title, "Interrupted assignment 1");
    reopened.recover_intents().unwrap();
    assert!(!receiver.has_changed().unwrap());
}

#[test]
fn recovery_conflicts_on_changed_task_even_when_externally_checked() {
    for already_checked in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (task, run_id, actor) = fixture.working(&root);
        fixture.apply(&actor, fixture.successful_result(&actor))
            .unwrap();
        let intent_id = fixture.pending_intent(&root, &task, &run_id);
        // A canonical external edit may race a durable acceptance intent; the
        // content API correctly refuses that intent rather than providing a bypass.
        let edited = {
            let locked = fixture.service.store.lock().unwrap();
            locked.tasks(&root).unwrap().update(&task.task_id, &task.task_revision,
                Some("Changed while acceptance was interrupted"), None).unwrap()
        };
        assert_ne!(edited.task_revision, task.task_revision);
        if already_checked {
            let locked = fixture.service.store.lock().unwrap();
            locked
                .tasks(&root)
                .unwrap()
                .check(&task.task_id, &edited.task_revision, true)
                .unwrap();
        }
        let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
        let mut receiver = reopened.subscribe();
        reopened.recover_intents().unwrap();
        let state = reopened.store.lock().unwrap().read().unwrap();
        let run = &state.runs[run_index(&state, &run_id).unwrap()];
        assert_eq!(run.stage, RunStage::Reported);
        assert!(run.close_reason.is_none());
        assert_eq!(state.task_intents.len(), 1);
        assert_eq!(state.task_intents[0].intent_id, intent_id);
        assert_eq!(state.task_intents[0].state, IntentState::Conflict);
        assert_eq!(
            state.task_intents[0].expected_task_revision,
            task.task_revision
        );
        assert_eq!(fixture.task(&root, &task.task_id).checked, already_checked);
        assert!(receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), state.revision);
        let recovered_revision = state.revision;
        reopened.recover_intents().unwrap();
        assert_eq!(
            reopened.store.lock().unwrap().read().unwrap().revision,
            recovered_revision
        );
        assert!(!receiver.has_changed().unwrap());
    }
}

#[test]
fn recovery_applies_unchanged_pending_acceptance_once() {
    for check_phase in 0..3 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (task, run_id, actor) = fixture.working(&root);
        // Pending from unchecked Markdown, crash after writing the exact check,
        // or acceptance explicitly requested on the already checked item.
        if check_phase == 2 {
            fixture
                .service
                .store
                .lock()
                .unwrap()
                .tasks(&root)
                .unwrap()
                .check(&task.task_id, &task.task_revision, true)
                .unwrap();
            fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
            fixture.pending_intent(&root, &fixture.task(&root, &task.task_id), &run_id);
        } else {
            fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
            fixture.pending_intent(&root, &task, &run_id);
            if check_phase == 1 {
                fixture
                    .service
                    .store
                    .lock()
                    .unwrap()
                    .tasks(&root)
                    .unwrap()
                    .check(&task.task_id, &task.task_revision, true)
                    .unwrap();
            }
        }
        let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
        let mut receiver = reopened.subscribe();
        reopened.recover_intents().unwrap();
        assert!(fixture.task(&root, &task.task_id).checked);
        assert_eq!(
            fixture.run(&run_id).close_reason,
            Some(CloseReason::Accepted)
        );
        assert!(fixture.state().task_intents.is_empty());
        let revision = fixture.state().revision;
        assert!(receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), revision);
        reopened.recover_intents().unwrap();
        assert_eq!(fixture.state().revision, revision);
        assert!(!receiver.has_changed().unwrap());
    }
}

#[test]
fn invalid_credentials_never_publish_initialized_or_result_evidence() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, actor) = fixture.working(&root);
    let foreign = fixture.root();
    let foreign_actor = fixture.launch_and_bind(&foreign);
    let Actor::Agent(foreign_caller) = foreign_actor else { unreachable!() };
    for fence in 0..4 {
        let mut denied = actor.clone();
        let caller = caller_mut(&mut denied);
        match fence {
            0 => caller.auth.credential_id = id(),
            1 => caller.auth.secret = foreign_caller.auth.secret.clone(),
            2 => caller.auth.attachment_id = id(),
            3 => caller.auth.attachment_epoch += 1,
            _ => unreachable!(),
        }
        for action in [
            report(ReportKind::Initialized, None, Some("Invalid diagnostic")),
            fixture.successful_result(&actor),
        ] {
            let before = fixture.state_bytes();
            assert!(fixture.apply(&denied, action).is_err());
            assert_eq!(fixture.state_bytes(), before);
        }
    }
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    assert!(fixture.run(&run_id).result.is_none());
    assert!(!fixture.task(&root, &task.task_id).checked);
}

#[test]
fn offline_ordinary_updates_use_credentials_without_physical_provenance() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, mut actor) = fixture.working(&root);
    let before = fixture.run(&run_id);
    let caller = caller_mut(&mut actor);
    caller.provenance = None;
    caller.endpoint_identity = "unreachable-endpoint".into();
    caller.workspace_id.clear();
    caller.tab_id.clear();
    caller.pane_id.clear();
    caller.boot_id = None;
    caller.terminal_id = None;
    caller.native_session_id = None;
    caller.env_run = None;
    caller.omp_session_id = None;
    caller.main_omp_session_id = None;
    caller.agent_kind = None;
    caller.actual_agent_kind = None;
    caller.process = None;
    let edited = task_result(fixture.apply(&actor, OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: task.task_id.clone(),
        expected_task_revision: task.task_revision,
        title: None, description: Some("Offline durable job edit".into()),
    }).unwrap());
    assert_eq!(edited.description, "Offline durable job edit");
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    let after = fixture.run(&run_id);
    assert_eq!(after.attachment.as_ref().unwrap().epoch, before.attachment.as_ref().unwrap().epoch);
    assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
    assert_eq!(serde_json::to_value(after.attachment).unwrap(), serde_json::to_value(before.attachment).unwrap());
}

#[test]
fn conversation_rollover_does_not_replace_epoch_credentials_or_checkout_claim() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, run_id, mut actor) = fixture.working(&root);
    let before = fixture.run(&run_id);
    let caller = caller_mut(&mut actor);
    let credential_id = caller.auth.credential_id.clone();
    caller.omp_session_id = Some("rolled-over-main".into());
    caller.main_omp_session_id = caller.omp_session_id.clone();
    let action = OrchestrationAction::RunAttach {
        operation_id: id(),
        attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: "rolled-over-main".into(),
    };
    fixture.apply(&actor, action).unwrap();
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    let after = fixture.run(&run_id);
    let attachment = after.attachment.as_ref().unwrap();
    assert_eq!(attachment.epoch, before.attachment.as_ref().unwrap().epoch);
    assert_eq!(attachment.attachment_id, before.attachment.as_ref().unwrap().attachment_id);
    assert_eq!(attachment.main_session_id.as_deref(), Some("rolled-over-main"));
    assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
    assert!(fixture.state().credentials.iter().any(|record| record.credential_id == credential_id
        && record.state == credentials::CredentialState::Active));
}


fn review_error() -> ErrorResponse {
    ErrorResponse {
        code: "launch_identity_conflict".into(),
        message: "Fresh runtime proof conflicts with the recorded launch".into(),
    }
}

// Review preserves current authority, incarnation, and worker evidence.
fn assert_review_fields_only(before: &Run, after: &Run) {
    assert_eq!(after.run_id, before.run_id);
    assert_eq!(after.root_id, before.root_id);
    assert_eq!(after.task_id, before.task_id);
    assert_eq!(after.attempt, before.attempt);
    assert_eq!(after.stage, before.stage);
    assert_eq!(after.close_reason, before.close_reason);
    assert_eq!(after.brief, before.brief);
    assert_eq!(after.task_revision_at_start, before.task_revision_at_start);
    assert_eq!(after.cleanup_policy, before.cleanup_policy);
    assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
    assert_eq!(serde_json::to_value(&after.attachment).unwrap(), serde_json::to_value(&before.attachment).unwrap());
    assert_eq!(serde_json::to_value(&after.setup).unwrap(), serde_json::to_value(&before.setup).unwrap());
    assert_eq!(serde_json::to_value(&after.last_report).unwrap(), serde_json::to_value(&before.last_report).unwrap());
    assert_eq!(serde_json::to_value(&after.result).unwrap(), serde_json::to_value(&before.result).unwrap());
    assert_eq!(after.source_output, before.source_output);
    assert_eq!(after.integration, before.integration);
    let (Some(before), Some(after)) = (&before.dispatch, &after.dispatch) else {
        assert_eq!(after.dispatch.is_some(), before.dispatch.is_some());
        return;
    };
    assert_eq!(after.launch_tag, before.launch_tag);
    assert_eq!(after.endpoint_identity, before.endpoint_identity);
    assert_eq!(after.launch_attempt, before.launch_attempt);
    assert_eq!(after.agent_started, before.agent_started);
}



fn advance_worker(fixture: &Fixture, _run_id: &str, actor: &Actor, stage: RunStage) {
    fixture.initialized(actor);
    if stage == RunStage::Reported {
        fixture.apply(actor, fixture.successful_result(actor))
            .unwrap();
    }
}

#[test]
fn healthy_physical_review_retains_missing_extension_until_actual_run_attach() {
    let fixture = Fixture::new();
    let run_id = fixture.root();
    fixture.setup(&run_id);
    let actor = fixture.launch_tab(&run_id);
    fixture.service.record_launch_pending(&fixture.run(&run_id)).unwrap();
    let Actor::Agent(caller) = &actor else { unreachable!() };
    // Host-observed main provenance exists before the extension commits RunAttach.
    fixture.seed_run(&run_id, |run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.main_session_id = caller.omp_session_id.clone();
        attachment.process = caller.process.clone();
    });
    let attachment = fixture.run(&run_id).attachment.unwrap();
    fixture.service.record_launch_verified(&fixture.run(&run_id)).unwrap();
    let retained = fixture.run(&run_id);
    assert_eq!(retained.stage, RunStage::Starting);
    let current = retained.attachment.as_ref().unwrap();
    assert_eq!(current.attachment_id, attachment.attachment_id);
    assert_eq!(current.epoch, attachment.epoch);
    assert_eq!(current.process, attachment.process);
    assert_eq!(current.state, AttachmentState::Disconnected);
    assert_eq!(retained.dispatch.as_ref().unwrap().error.as_ref().unwrap().code, "current_extension_unavailable");
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&run_id));
    let messages = serde_json::to_value(fixture.state().messages).unwrap();
    fixture.service.record_launch_verified(&retained).unwrap();
    let observed_again = fixture.run(&run_id);
    let current = observed_again.attachment.as_ref().unwrap();
    assert_eq!(observed_again.stage, RunStage::Starting);
    assert_eq!(current.state, AttachmentState::Disconnected);
    assert_eq!(current.attachment_id, attachment.attachment_id);
    assert_eq!(current.epoch, attachment.epoch);
    assert_eq!(current.process, attachment.process);
    assert_eq!(current.shell, attachment.shell);
    assert_eq!(observed_again.dispatch.as_ref().unwrap().launch_attempt,
        retained.dispatch.as_ref().unwrap().launch_attempt);
    assert_eq!(observed_again.dispatch.as_ref().unwrap().launch_tag,
        retained.dispatch.as_ref().unwrap().launch_tag);
    assert_eq!(observed_again.dispatch.as_ref().unwrap().error.as_ref().unwrap().code,
        "current_extension_unavailable");
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&run_id));
    assert_eq!(serde_json::to_value(fixture.state().messages).unwrap(), messages);
    fixture.apply(&actor, OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    }).unwrap();
    let reattached = fixture.run(&run_id);
    assert_eq!(reattached.stage, RunStage::Active);
    assert_eq!(reattached.attachment.as_ref().unwrap().state, AttachmentState::Connected);
    assert_eq!(reattached.attachment.as_ref().unwrap().attachment_id, attachment.attachment_id);
    assert_eq!(reattached.attachment.as_ref().unwrap().epoch, attachment.epoch);
    assert!(reattached.dispatch.as_ref().unwrap().error.is_none());
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&run_id));
    assert_eq!(fixture.state().messages.iter().filter(|message|
        message.to_run_id == run_id && message.kind == MessageKind::SupervisorBrief).count(), 1);
}

#[test]
fn first_launch_proof_preserves_fast_same_launch_progress_and_repeated_proof_is_no_write() {
    for stage in [
        RunStage::Working,
        RunStage::Reported,
        RunStage::Active,
    ] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let run_id = if stage == RunStage::Active {
            fixture.setup(&root);
            root
        } else {
            let task = fixture.create_task(&root, "Fast same-launch progress");
            let run_id = fixture.start(&root, &task);
            fixture.setup(&run_id);
            run_id
        };
        let actor = fixture.launch_tab(&run_id);
        fixture
            .service
            .record_launch_pending(&fixture.run(&run_id))
            .unwrap();
        let Actor::Agent(caller) = &actor else {
            unreachable!();
        };
        fixture
            .apply(
                &actor,
                OrchestrationAction::RunAttach { operation_id: id(), attachment_id: caller.auth.attachment_id.clone(), expected_attachment_epoch: caller.auth.attachment_epoch, main_session_id: caller.omp_session_id.clone().unwrap() },
            )
            .unwrap();
        let reviewed = fixture.run(&run_id);
        // Model same-incarnation report progress while runtime proof is read.
        fixture.seed_run(&run_id, |run| {
            run.stage = if stage == RunStage::Active {
                RunStage::Active
            } else {
                RunStage::Working
            };
        });
        if stage == RunStage::Active {
            fixture.apply(&actor, fixture.successful_result(&actor))
                .unwrap();
        } else {
            advance_worker(&fixture, &run_id, &actor, stage);
        }
        let before = fixture.run(&run_id);
        assert!(!before.dispatch.as_ref().unwrap().agent_started);
        let original_messages = serde_json::to_value(fixture.state().messages).unwrap();
        let revision = fixture.state().revision;
        assert_eq!(
            fixture.service.record_launch_verified(&reviewed).unwrap(),
            revision + 1
        );
        let after = fixture.run(&run_id);
        assert_eq!(after.stage, stage);
        assert_eq!(after.task_id, before.task_id);
        assert_eq!(after.root_id, before.root_id);
        assert_eq!(after.attempt, before.attempt);
        assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
        assert_eq!(serde_json::to_value(&after.setup).unwrap(), serde_json::to_value(&before.setup).unwrap());
        assert_eq!(serde_json::to_value(&after.result).unwrap(), serde_json::to_value(&before.result).unwrap());
        let before_attachment = before.attachment.as_ref().unwrap();
        let after_attachment = after.attachment.as_ref().unwrap();
        assert_eq!(after_attachment.attachment_id, before_attachment.attachment_id);
        assert_eq!(after_attachment.epoch, before_attachment.epoch);
        assert_eq!(after_attachment.process, before_attachment.process);
        assert_eq!(after_attachment.main_session_id, before_attachment.main_session_id);
        assert_eq!(after_attachment.state, AttachmentState::Connected);
        assert!(after.dispatch.as_ref().unwrap().agent_started);
        assert_eq!(
            after.dispatch.as_ref().unwrap().step,
            DispatchStep::Launched
        );
        let kind = if stage == RunStage::Active {
            MessageKind::SupervisorBrief
        } else {
            MessageKind::WorkBrief
        };
        let messages = fixture.state().messages;
        let retained: Vec<_> = messages
            .into_iter()
            .filter(|message| !(message.to_run_id == run_id && message.kind == kind))
            .collect();
        assert_eq!(serde_json::to_value(retained).unwrap(), original_messages);
        // Repeated physical proof never reissues or reinitializes launch work.
        let revision = fixture.state().revision;
        let bytes = fixture.state_bytes();
        let receiver = fixture.service.revision.subscribe();
        assert_eq!(
            fixture.service.record_launch_verified(&reviewed).unwrap(),
            revision
        );
        assert_eq!(fixture.state_bytes(), bytes);
        assert!(!receiver.has_changed().unwrap());
    }
}

fn assert_agent_decision(message: &Message, run_id: &str) {
    assert!(matches!(&message.from, ActorRef::Run { run_id: from } if from == run_id));
    assert!(!message.stale);
}

fn assert_proof_discarded(fixture: &Fixture, reviewed: &Run) {
    let revision = fixture.state().revision;
    let bytes = fixture.state_bytes();
    let receiver = fixture.service.revision.subscribe();
    assert_eq!(
        fixture.service.record_launch_verified(reviewed).unwrap(),
        revision
    );
    assert_eq!(
        fixture.state_bytes(),
        bytes,
        "Obsolete proof must not write"
    );
    assert!(
        !receiver.has_changed().unwrap(),
        "Obsolete proof must not notify"
    );
}

#[test]
fn pending_launch_ack_and_independent_bind_do_not_promote_root_or_worker() {
    for worker in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let run_id = if worker {
            let task = fixture.create_task(&root, "Pending initialization");
            let run_id = fixture.start(&root, &task);
            fixture.setup(&run_id);
            run_id
        } else {
            root
        };
        let actor = fixture.launch_tab(&run_id);
        let unbound = fixture.run(&run_id);
        assert_eq!(unbound.stage, RunStage::Starting);
        assert!(unbound.attachment.as_ref().unwrap().main_session_id.is_none());
        assert_proof_discarded(&fixture, &unbound);
        let Actor::Agent(caller) = &actor else {
            unreachable!()
        };
        fixture
            .apply(
                &actor,
                OrchestrationAction::RunAttach { operation_id: id(), attachment_id: caller.auth.attachment_id.clone(), expected_attachment_epoch: caller.auth.attachment_epoch, main_session_id: caller.omp_session_id.clone().unwrap() },
            )
            .unwrap();
        assert_eq!(fixture.run(&run_id).stage, RunStage::Starting);
        let revision = fixture.state().revision;
        // SDK binding can win the race with accepted agent.start ACK. The old
        // unbound request may record Pending, but is never sufficient proof.
        assert_eq!(
            fixture.service.record_launch_pending(&unbound).unwrap(),
            revision + 1
        );
        let pending = fixture.run(&run_id);
        assert_eq!(
            pending.dispatch.as_ref().unwrap().step,
            DispatchStep::LaunchPending
        );
        assert!(!pending.dispatch.as_ref().unwrap().agent_started);
        assert_eq!(pending.stage, RunStage::Starting);
        assert_eq!(pending.attachment.as_ref().unwrap().main_session_id, caller.omp_session_id);
        assert!(fixture.state().messages.iter().all(|message| {
            message.to_run_id != run_id
                || !matches!(
                    message.kind,
                    MessageKind::SupervisorBrief | MessageKind::WorkBrief
                )
        }));
        assert_proof_discarded(&fixture, &unbound);
        let revision = fixture.state().revision;
        fixture.service.record_launch_verified(&pending).unwrap();
        let proven = fixture.run(&run_id);
        assert_eq!(fixture.state().revision, revision + 1);
        assert_eq!(
            proven.stage,
            if worker {
                RunStage::Working
            } else {
                RunStage::Active
            }
        );
        assert!(proven.dispatch.as_ref().unwrap().agent_started);
        assert_eq!(
            proven.dispatch.as_ref().unwrap().step,
            DispatchStep::Launched
        );
        let kind = if worker {
            MessageKind::WorkBrief
        } else {
            MessageKind::SupervisorBrief
        };
        let messages = fixture.state().messages;
        let briefs: Vec<_> = messages
            .iter()
            .filter(|message| message.to_run_id == run_id && message.kind == kind)
            .collect();
        assert_eq!(briefs.len(), 1);
        assert_eq!(briefs[0].text, proven.brief);
        assert_proof_discarded(&fixture, &proven);
    }
}

#[test]
fn launch_proof_fences_target_task_native_binding_and_every_receipt_field() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Exact launch proof");
    let run_id = fixture.start(&root, &task);
    fixture.setup(&run_id);
    let actor = fixture.launch_tab(&run_id);
    fixture
        .service
        .record_launch_pending(&fixture.run(&run_id))
        .unwrap();
    let Actor::Agent(caller) = &actor else {
        unreachable!()
    };
    fixture
        .apply(
            &actor,
            OrchestrationAction::RunAttach { operation_id: id(), attachment_id: caller.auth.attachment_id.clone(), expected_attachment_epoch: caller.auth.attachment_epoch, main_session_id: caller.omp_session_id.clone().unwrap() },
        )
        .unwrap();
    let reviewed = fixture.run(&run_id);
    for fence in 0..20 {
        let mut stale = reviewed.clone();
        match fence {
            0 => stale.root_id = id(),
            1 => stale.task_id = Some(id()),
            2 => stale.session_id = "different-session".into(),
            3 => stale.attempt += 1,
            4 => stale.dispatch.as_mut().unwrap().launch_attempt += 1,
            5 => stale.dispatch.as_mut().unwrap().launch_tag = Some("different-tag".into()),
            6 => {
                stale.dispatch.as_mut().unwrap().endpoint_identity =
                    Some("different-endpoint".into())
            }
            7 => stale.attachment.as_mut().unwrap().main_session_id = None,
            8 => stale.attachment.as_mut().unwrap().main_session_id = Some(String::new()),
            9 => stale.attachment.as_mut().unwrap().main_session_id = Some("other-native-main".into()),
            10 => stale.attachment.as_mut().unwrap().location = None,
            11 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().boot_id = Some("different-boot".into()),
            12 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().terminal_id = Some("different-terminal".into()),
            13 => {
                stale.attachment.as_mut().unwrap().location.as_mut().unwrap().native_session_id = Some("different-native".into())
            }
            14 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().endpoint_identity = "different-endpoint".into(),
            15 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().session_id = "different-session".into(),
            16 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().workspace_id = "different-space".into(),
            17 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().tab_id = "different-tab".into(),
            18 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().pane_id = "different-pane".into(),
            19 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().launch_tag = "different-tag".into(),
            _ => unreachable!(),
        }
        assert_proof_discarded(&fixture, &stale);
    }
    // Even matching captured/current corrupt receipts are not valid proof.
    for fence in 0..5 {
        fixture.seed_run(&run_id, |run| {
            *run = reviewed.clone();
            match fence {
                0 => run.attachment.as_mut().unwrap().main_session_id = Some(String::new()),
                1 => {
                    run.attachment.as_mut().unwrap().location.as_mut().unwrap().native_session_id =
                        Some("conflicting-native".into())
                }
                2 => run.attachment.as_mut().unwrap().location.as_mut().unwrap().launch_tag.clear(),
                3 => {
                    run.dispatch.as_mut().unwrap().endpoint_identity = Some("wrong-endpoint".into())
                }
                4 => run.attachment.as_mut().unwrap().location.as_mut().unwrap().session_id = "wrong-session".into(),
                _ => unreachable!(),
            }
        });
        assert_proof_discarded(&fixture, &fixture.run(&run_id));
    }
    fixture.seed_run(&run_id, |run| {
        *run = reviewed;
        // Herdr's optional native ID is not a required launch gate.
        run.attachment.as_mut().unwrap().location.as_mut().unwrap().native_session_id = None;
    });
    fixture
        .service
        .record_launch_verified(&fixture.run(&run_id))
        .unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
}

#[test]
fn late_initial_proof_cannot_revive_cancel_retry_native_bind_or_move() {
    for race in 0..4 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let actor = fixture.launch_tab(&root);
        fixture
            .service
            .record_launch_pending(&fixture.run(&root))
            .unwrap();
        let Actor::Agent(caller) = &actor else {
            unreachable!()
        };
        if race != 2 {
            fixture
                .apply(
                    &actor,
                    OrchestrationAction::RunAttach { operation_id: id(), attachment_id: caller.auth.attachment_id.clone(), expected_attachment_epoch: caller.auth.attachment_epoch, main_session_id: caller.omp_session_id.clone().unwrap() },
                )
                .unwrap();
        }
        let reviewed = fixture.run(&root);
        match race {
            0 => {
                fixture.operator(OrchestrationAction::CancelRun {
                    run_id: root.clone(),
                });
            }
            1 => {
                fixture
                    .service
                    .record_dispatch(
                        &root,
                        DispatchUpdate::Step {
                            step: DispatchStep::LaunchUnknown,
                            error: Some(review_error()),
                        },
                    )
                    .unwrap();
                fixture.resume_exited(&Actor::Operator(OperatorOrigin::Browser), &root);
                assert_eq!(
                    fixture.run(&root).dispatch.unwrap().launch_attempt,
                    reviewed.dispatch.as_ref().unwrap().launch_attempt + 1
                );
            }
            2 => {
                fixture
                    .apply(
                        &actor,
                        OrchestrationAction::RunAttach { operation_id: id(), attachment_id: caller.auth.attachment_id.clone(), expected_attachment_epoch: caller.auth.attachment_epoch, main_session_id: caller.omp_session_id.clone().unwrap() },
                    )
                    .unwrap();
            }
            3 => {
                let mut moved = actor.clone();
                let moved_caller = caller_mut(&mut moved);
                moved_caller.workspace_id = "moved-space".into();
                moved_caller.tab_id = "moved-tab".into();
                moved_caller.pane_id = "moved-pane".into();
                fixture.attach_observed(&moved, &root);
            }
            _ => unreachable!(),
        }
        assert_proof_discarded(&fixture, &reviewed);
        assert!(
            fixture
                .state()
                .messages
                .iter()
                .all(|message| message.kind != MessageKind::SupervisorBrief)
        );
    }
}

#[test]
fn linked_answer_retries_preserve_committed_receipts_after_new_question_and_recheck_authority() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let (_, run_id, worker) = fixture.working(&root);
    fixture.apply(&worker, report(ReportKind::NeedsInput, None, None)).unwrap();
    let q1 = fixture.run(&run_id).last_report.unwrap().message_id;
    let answer = OrchestrationAction::MessageSend { message_id: id(),
    to_run_id: run_id.clone(),
    kind: MessageKind::Answer,
    text: "Use the approved choice".into(), in_reply_to: Some(q1.clone()), original_task_revision: None, original_claim_id: None };
    let first = fixture.apply(&supervisor, answer.clone()).unwrap();
    let OrchestrationActionResult::Message { seq: first_seq, duplicate: false, .. } = first.result else {
        panic!("expected newly committed answer");
    };
    fixture.apply(&worker, report(ReportKind::NeedsInput, None, None)).unwrap();
    let q2 = fixture.run(&run_id).last_report.unwrap().message_id;
    let mut late = answer.clone();
    if let OrchestrationAction::MessageSend { message_id, .. } = &mut late {
        *message_id = id();
    }
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, late), "question_not_current");
    assert_eq!(fixture.state_bytes(), bytes);
    let inbox_before = serde_json::to_value(fixture.state().messages).unwrap();
    let retry = fixture.apply(&supervisor, answer.clone()).unwrap();
    assert!(matches!(retry.result, OrchestrationActionResult::Message { seq, duplicate: true, .. } if seq == first_seq));
    assert_eq!(serde_json::to_value(fixture.state().messages).unwrap(), inbox_before);
    assert_eq!(fixture.run(&run_id).last_report.unwrap().message_id, q2);
    for link in [Some(q2), None, Some(String::new())] {
        let mut changed = answer.clone();
        if let OrchestrationAction::MessageSend { in_reply_to, .. } = &mut changed {
            *in_reply_to = link;
        }
        let bytes = fixture.state_bytes();
        assert!(fixture.apply(&supervisor, changed).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    let mut obsolete = supervisor.clone();
    caller_mut(&mut obsolete).auth.attachment_epoch += 1;
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&obsolete, answer), "job_unauthorized");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn bound_top_root_starts_answers_sends_back_accepts_and_cancels() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "Root-managed worker");
    let run_id = run_result(fixture.apply(&supervisor, fixture.start_action(&root, &task)).unwrap()).0;
    fixture.setup(&run_id);
    let worker = fixture.launch_and_bind(&run_id);
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    let native = fixture.run(&root).attachment.unwrap().main_session_id.unwrap();
    fixture
        .apply(&worker, report(ReportKind::NeedsInput, None, None))
        .unwrap();
    let question_id = fixture.run(&run_id).last_report.unwrap().message_id;
    let answer_id = id();
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::MessageSend { message_id: answer_id.clone(),
            to_run_id: run_id.clone(),
            kind: MessageKind::Answer,
            text: "Use the established task scope; no additional decision is missing".into(), in_reply_to: Some(question_id.clone()), original_task_revision: None, original_claim_id: None },
        )
        .unwrap();
    let answer = fixture
        .state()
        .messages
        .into_iter()
        .find(|message| message.message_id == answer_id)
        .unwrap();
    assert_agent_decision(&answer, &root);
    assert_eq!(answer.in_reply_to.as_deref(), Some(question_id.as_str()));
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    fixture.apply(&worker, fixture.successful_result(&worker))
        .unwrap();
    assert!(!fixture.task(&root, &task.task_id).checked);
    fixture
        .apply(&worker, report(ReportKind::NeedsInput, None, None))
        .unwrap();
    let sendback_question = fixture.run(&run_id).last_report.unwrap().message_id;
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::SendBack {
                run_id: run_id.clone(),
                text: "Add explicit evidence for the second acceptance criterion".into(),
            },
        )
        .unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    assert!(fixture.run(&run_id).result.is_none());
    let sendback = fixture
        .state()
        .messages
        .into_iter()
        .find(|message| {
            message.to_run_id == run_id
                && message.kind == MessageKind::Answer
                && message.text == "Add explicit evidence for the second acceptance criterion"
        })
        .unwrap();
    assert_agent_decision(&sendback, &root);
    assert!(sendback.in_reply_to.is_none());
    let state = fixture.state();
    let run = state.runs.iter().find(|run| run.run_id == run_id).unwrap();
    let inbox = state.messages.iter().filter(|message| message.to_run_id == run_id).collect::<Vec<_>>();
    let question = projection::question_status(run, &inbox, &state.messages).unwrap();
    assert_eq!(question.question_message_id, sendback_question);
    assert!(matches!(question.receipt, QuestionReceipt::Unresolved));
    fixture.apply(&worker, fixture.successful_result(&worker))
        .unwrap();
    let edited = task_result(
        fixture
            .apply(
                &supervisor,
                OrchestrationAction::TaskUpdate {
                    root_id: root.clone(),
                    task_id: task.task_id.clone(),
                    expected_task_revision: task.task_revision.clone(),
                    title: None,
                    description: Some("Canonical task body with reviewed evidence requirements".into()),
                },
            )
            .unwrap(),
    );
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(
            &supervisor,
            OrchestrationAction::Accept {
                run_id: run_id.clone(),
                expected_task_revision: task.task_revision,
            },
        ),
        "task_revision_conflict",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    assert_code(fixture.apply(&supervisor, OrchestrationAction::Accept {
        run_id: run_id.clone(), expected_task_revision: edited.task_revision.clone(),
    }), "source_output_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.apply(&supervisor, OrchestrationAction::SendBack {
        run_id: run_id.clone(), text: "Report output for the updated canonical scope".into(),
    }).unwrap();
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    let result_id = fixture.run(&run_id).result.unwrap().message_id;
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::Accept {
                run_id: run_id.clone(),
                expected_task_revision: edited.task_revision.clone(),
            },
        )
        .unwrap();
    assert!(fixture.task(&root, &task.task_id).checked);
    let accepted = fixture.run(&run_id);
    assert_eq!(accepted.close_reason, Some(CloseReason::Accepted));
    assert_eq!(accepted.stage, RunStage::Closed);
    assert!(fixture.state().task_intents.is_empty());
    let requested = accepted
        .annotations
        .iter()
        .find(|annotation| annotation.text.starts_with("Acceptance requested"))
        .unwrap();
    assert!(matches!(&requested.by, ActorRef::Run { run_id } if run_id == &root));
    assert!(requested.text.contains(&result_id));
    assert!(requested.text.contains(&edited.task_revision));
    assert!(requested.text.contains(&native));
    assert!(accepted.annotations.iter().any(|annotation| {
        annotation.text.starts_with("Accepted Result")
            && matches!(&annotation.by, ActorRef::Run { run_id } if run_id == &root)
    }));
    let (cancel_task, cancelled, _) = fixture.working(&root);
    let (_, survivor, _) = fixture.working(&cancelled);
    let survivor_before = fixture.run(&survivor);
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::CancelRun {
                run_id: cancelled.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        fixture.run(&cancelled).close_reason,
        Some(CloseReason::Cancelled)
    );
    assert!(!fixture.task(&root, &cancel_task.task_id).checked);
    assert_review_fields_only(&survivor_before, &fixture.run(&survivor));
    let cancel = fixture
        .state()
        .messages
        .into_iter()
        .find(|message| {
            message.to_run_id == cancelled && message.kind == MessageKind::CancelRequest
        })
        .unwrap();
    assert_agent_decision(&cancel, &root);
}

#[test]
fn management_requires_root_credentials_not_worker_child_or_forged_identity() {
    for operation in [0, 2, 3, 4, 5, 6] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let supervisor = fixture.launch_and_bind(&root);
        let task = fixture.create_task(&root, "Authority boundary");
        let run_id = fixture.start(&root, &task);
        fixture.setup(&run_id);
        let worker = fixture.launch_and_bind(&run_id);
        if matches!(operation, 2 | 3) {
            fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
        }
        let action = match operation {
            0 => fixture.start_action(&root, &fixture.create_task(&root, "Denied start")),
            2 => OrchestrationAction::Accept {
                run_id: run_id.clone(),
                expected_task_revision: task.task_revision,
            },
            3 => OrchestrationAction::SendBack {
                run_id: run_id.clone(),
                text: "Review correction".into(),
            },
            4 => OrchestrationAction::CancelRun {
                run_id: run_id.clone(),
            },
            5 => OrchestrationAction::MessageSend { message_id: id(),
            to_run_id: run_id.clone(),
            kind: MessageKind::Answer,
            text: "Answer".into(), in_reply_to: None, original_task_revision: None, original_claim_id: None },
            6 => OrchestrationAction::ReconcileRun {
                run_id: run_id.clone(),
                recovery: None,
            },
            _ => unreachable!(),
        };
        let other_root = fixture.root();
        let outsider = fixture.launch_and_bind(&other_root);
        let (_, branch_id, branch_actor) = fixture.working(&root);
        let mut inherited_env = branch_actor.clone();
        caller_mut(&mut inherited_env).env_run = Some((root.clone(), 1));
        let mut denied = vec![outsider, branch_actor.clone(), inherited_env];
        denied.push(worker);
        for fence in 0..3 {
            let mut actor = supervisor.clone();
            let caller = caller_mut(&mut actor);
            match fence {
                0 => caller.auth.credential_id = id(),
                1 => caller.auth.attachment_id = id(),
                2 => caller.auth.attachment_epoch += 1,
                _ => unreachable!(),
            }
            denied.push(actor);
        }
        let mut branch_subagent = fixture.child_actor(&branch_actor, &branch_id, "worker-child");
        // Physical claims of being main cannot enlarge this narrowed child credential.
        let caller = caller_mut(&mut branch_subagent);
        caller.agent_kind = Some(AgentKind::Main);
        caller.subagent_id = None;
        caller.env_run = Some((root.clone(), 1));
        caller.provenance = None;
        denied.push(branch_subagent);
        for actor in denied {
            let bytes = fixture.state_bytes();
            let failure = fixture.apply(&actor, action.clone()).unwrap_err();
            assert!(
                matches!(
                    failure.code.as_str(),
                    "actor_forbidden" | "caller_mismatch" | "session_mismatch" | "attempt_stale" | "job_unauthorized"
                ),
                "Operation {operation} unexpectedly failed outside authority fences: {failure:?}"
            );
            assert_eq!(
                fixture.state_bytes(),
                bytes,
                "Denied operation {operation} must not write"
            );
        }
        let valid_root = fixture.run(&root);
        for fence in 0..7 {
            fixture.seed_run(&root, |run| {
                *run = valid_root.clone();
                match fence {
                    0 => run.kind = RunKind::Worker,
                    1 => run.stage = RunStage::Starting,
                    2 => run.stage = RunStage::Closed,
                    3 => run.parent_run_id = Some(branch_id.clone()),
                    4 => run.root_id = other_root.clone(),
                    5 => run.attachment.as_mut().unwrap().epoch += 1,
                    6 => run.attachment.as_mut().unwrap().state = AttachmentState::Revoked,
                    _ => unreachable!(),
                }
            });
            let bytes = fixture.state_bytes();
            let denied = fixture.apply(&supervisor, action.clone());
            assert!(denied.is_err());
            assert_eq!(
                fixture.state_bytes(),
                bytes,
                "Malformed root must not gain management authority"
            );
        }
    }
}

#[test]
fn supervisor_recovers_only_strict_descendants() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let (_, branch, branch_actor) = fixture.working(&root);
    let (_, nested, _) = fixture.working(&branch);
    let other_root = fixture.root();
    let (_, outsider, _) = fixture.working(&other_root);
    for target in [&root, &other_root, &outsider] {
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.apply(
                &supervisor,
                OrchestrationAction::CancelRun {
                    run_id: target.clone(),
                },
            ),
            "actor_forbidden",
        );
        assert_eq!(fixture.state_bytes(), bytes);
    }
    let reviewed = fixture.run(&nested);
    let blocked = fixture.apply(&supervisor, fixture.resume_action(&nested)).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = blocked.result else { unreachable!() };
    assert_eq!(resume.resume.phase, ResumePhase::Blocked);
    assert_eq!(fixture.run(&nested).attachment.as_ref().unwrap().epoch,
        reviewed.attachment.as_ref().unwrap().epoch);
    for target in [&root, &other_root, &outsider, &branch] {
        let actor = if target == &branch { &branch_actor } else { &supervisor };
        for operation in 0..2 {
            let bytes = fixture.state_bytes();
            let result = if operation == 0 {
                fixture.apply(actor, OrchestrationAction::ReconcileRun { run_id: target.clone(), recovery: None })
            } else {
                fixture.apply(actor, fixture.resume_action(target))
            };
            if operation == 1 && (target == &root || target == &branch) {
                assert!(matches!(result.unwrap().result, OrchestrationActionResult::RunResumed { .. }));
            } else {
                assert_code(result, "actor_forbidden");
                assert_eq!(fixture.state_bytes(), bytes);
            }
        }
    }
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, OrchestrationAction::ReconcileRun {
        run_id: nested.clone(), recovery: Some(cockpit_protocol::projects::WorkspaceRecoveryAction::RetryEnvironment),
    }), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.resume_exited(&supervisor, &nested);
    let retried = fixture.run(&nested);
    assert_eq!(retried.dispatch.as_ref().unwrap().launch_attempt, reviewed.dispatch.as_ref().unwrap().launch_attempt + 1);
    assert_eq!(retried.stage, RunStage::Starting);
    assert!(retried.attachment.as_ref().unwrap().location.is_none());
    assert_eq!(retried.task_id, reviewed.task_id);
    assert_eq!(retried.root_id, reviewed.root_id);
    assert_eq!(retried.brief, reviewed.brief);
    assert_eq!(serde_json::to_value(&retried.setup).unwrap(), serde_json::to_value(&reviewed.setup).unwrap());
    assert_eq!(serde_json::to_value(&retried.result).unwrap(), serde_json::to_value(&reviewed.result).unwrap());
    assert_eq!(fixture.state().operations.last().unwrap().actor_run_id.as_deref(), Some(root.as_str()));
    fixture.seed_run(&nested, |run| run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, OrchestrationAction::ReconcileRun {
        run_id: nested.clone(), recovery: None,
    }), "invalid_stage");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::CancelRun {
                run_id: nested.clone(),
            },
        )
        .unwrap();
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(
            &supervisor,
            OrchestrationAction::MessageSend { message_id: id(),
            to_run_id: nested,
            kind: MessageKind::Answer,
            text: "Too late".into(), in_reply_to: None, original_task_revision: None, original_claim_id: None },
        ),
        "actor_forbidden",
    );
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn explicitly_adopted_actual_main_root_has_management_authority() {
    let fixture = Fixture::new();
    let seed = fixture.root();
    let mut actor = fixture.launch_tab(&seed);
    let caller = caller_mut(&mut actor);
    caller.env_run = None;
    caller.pane_id = "unclaimed-pane".into();
    caller.tab_id = "unclaimed-tab".into();
    caller.terminal_id = Some("unclaimed-terminal".into());
    caller.native_session_id = Some("adopted-native".into());
    caller.omp_session_id = Some("adopted-native".into());
    caller.main_omp_session_id = Some("adopted-native".into());
    caller.process = Some(live_process());
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let bootstrap = BootstrapCaller {
        endpoint_identity: caller.endpoint_identity.clone(), session_id: caller.session_id.clone(),
        workspace_id: caller.workspace_id.clone(), tab_id: caller.tab_id.clone(), pane_id: caller.pane_id.clone(),
        boot_id: caller.boot_id.clone(), terminal_id: caller.terminal_id.clone(),
        native_session_id: caller.native_session_id.clone(), omp_session_id: caller.omp_session_id.clone(),
        agent_kind: caller.agent_kind, actual_agent_kind: caller.actual_agent_kind.clone(), process: caller.process.clone(),
    };
    let mut shell = bootstrap.clone();
    shell.actual_agent_kind = Some(NativeAgentKind::Other("shell".into()));
    let bytes = fixture.state_bytes();
    assert!(fixture.apply(
        &Actor::Bootstrap(shell),
        OrchestrationAction::RunAdopt { label: "Not OMP".into() },
    ).is_err());
    assert_eq!(fixture.state_bytes(), bytes);
    let root = run_result(
        fixture
            .apply(
                &Actor::Bootstrap(bootstrap),
                OrchestrationAction::RunAdopt {
                    label: "Actual main explicitly adopted".into(),
                },
            )
            .unwrap(),
    )
    .0;
    assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().state, AttachmentState::Connecting);
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
    caller_mut(&mut actor).env_run = Some((root.clone(), 1));
    caller_mut(&mut actor).auth = fixture.auth(&root, JobRole::Root, None);
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root),
        "Adoption and staged credentials alone do not prove a current loaded extension");
    let Actor::Agent(caller) = &actor else { unreachable!() };
    fixture.apply(&actor, OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    }).unwrap();
    assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().state, AttachmentState::Connected);
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
    assert_eq!(fixture.run(&root).kind, RunKind::Adopted);
    let task = fixture.create_task(&root, "Adopted root task");
    let run_id = run_result(fixture.apply(&actor, fixture.start_action(&root, &task)).unwrap()).0;
    assert_eq!(fixture.run(&run_id).parent_run_id.as_deref(), Some(root.as_str()));
    let edited = task_result(fixture.apply(&actor, OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
        title: None, description: Some("Adopted native root owns canonical content".into()),
    }).unwrap());
    assert_eq!(edited.description, "Adopted native root owns canonical content");
    let follow = task_result(fixture.apply(&actor, OrchestrationAction::TaskCreate {
        root_id: root.clone(), task_id: id(), title: "Adopted root follow-up".into(), description: String::new(),
        depends_on: vec![edited.task_id.clone()], follow_up_of: Some(edited.task_id.clone()),
        expected_doc_revision: Some(task_document_revision(&fixture, &root)), source_revision: Some(edited.task_revision),
    }).unwrap());
    assert_eq!(follow.follow_up_of.as_deref(), Some(edited.task_id.as_str()));
}

#[test]
fn acceptance_recovery_cannot_substitute_a_new_result_for_the_reviewed_receipt() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let (task, run_id, worker) = fixture.working(&root);
    fixture.apply(&worker, fixture.successful_result(&worker))
        .unwrap();
    let reviewed_result = fixture.run(&run_id).result.unwrap().message_id;
    let supervisor_native = fixture.run(&root).attachment.unwrap().main_session_id;
    let intent_id = fixture.pending_intent(&root, &task, &run_id);
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let intent = state
            .task_intents
            .iter_mut()
            .find(|intent| intent.intent_id == intent_id)
            .unwrap();
        intent.origin = Some(DecisionOrigin::Supervisor);
        intent.supervisor_run_id = Some(root.clone());
        intent.omp_session_id = supervisor_native;
        intent.result_message_id = Some(reviewed_result);
        locked.save(&mut state).unwrap();
    }
    for action in [
        OrchestrationAction::SendBack {
            run_id: run_id.clone(),
            text: "Must resolve pending exact acceptance first".into(),
        },
        OrchestrationAction::CancelRun {
            run_id: run_id.clone(),
        },
    ] {
        let bytes = fixture.state_bytes();
        assert_code(fixture.apply(&supervisor, action), "intent_conflict");
        assert_eq!(fixture.state_bytes(), bytes);
    }
    // Model concurrent replacement from a previously interrupted recovery.
    fixture.seed_run(&run_id, |run| {
        run.result.as_mut().unwrap().message_id = id()
    });
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    reopened.recover_intents().unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
    assert!(!fixture.task(&root, &task.task_id).checked);
    assert_eq!(fixture.state().task_intents[0].state, IntentState::Conflict);
    assert_eq!(fixture.state().task_intents[0].intent_id, intent_id);
}

#[test]
fn child_reports_and_parent_inbox_delivery_preserve_nonnull_parent_receipts() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, parent, parent_actor) = fixture.working(&root);
    let (_, child, child_actor) = fixture.working(&parent);
    fixture.apply(&parent_actor, fixture.successful_result(&parent_actor))
        .unwrap();
    let parent_before = fixture.run(&parent);
    let parent_result = serde_json::to_value(parent_before.result.as_ref().unwrap()).unwrap();
    let assert_parent_receipts = || {
        let after = fixture.run(&parent);
        assert_eq!(after.stage, RunStage::Reported);
        assert_eq!(
            serde_json::to_value(after.result.as_ref().unwrap()).unwrap(),
            parent_result
        );
        assert_eq!(
            serde_json::to_value(after.last_report.as_ref().unwrap()).unwrap(),
            parent_result
        );
    };
    let mut report_ids = Vec::new();
    for (kind, summary, outcome) in [
        (ReportKind::Progress, "Child completed its first step", None),
        (ReportKind::NeedsInput, "Which deployment target should the child use?", None),
        (ReportKind::Result, "Child completed the selected deployment", Some(ReportOutcome::Succeeded)),
    ] {
        let message_id = id();
        fixture
            .apply(
                &child_actor,
                OrchestrationAction::Report {
                    message_id: message_id.clone(),
                    kind,
                    outcome,
                    summary: summary.into(),
                    plan: None,
                    source_output: (kind == ReportKind::Result).then(|| SourceOutput::NonCode {
                        result_message_id: message_id.clone(),
                        task_revision: fixture.task(&root, fixture.run(&child).task_id.as_deref().unwrap()).task_revision,
                        artifacts: Vec::new(), focused_checks: Vec::new(),
                    }),
                    to_run_id: None,
                    original_task_revision: None,
                    original_claim_id: None,
                },
            )
            .unwrap();
        assert_parent_receipts();
        let state = fixture.state();
        let delivered = state
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .unwrap();
        assert_agent_decision(delivered, &child);
        assert_eq!(delivered.to_run_id, parent);
        assert_eq!(delivered.kind, MessageKind::Report);
        assert_eq!(delivered.from_subagent_id, None);
        assert_eq!(delivered.stage, DeliveryStage::Stored);
        let receipt = delivered.report.as_ref().unwrap();
        assert_eq!(receipt.kind, kind);
        assert_eq!(receipt.summary, summary);
        assert_eq!(receipt.outcome, outcome);
        let child_after = fixture.run(&child);
        assert_eq!(child_after.last_report.as_ref().unwrap().message_id, message_id);
        if kind == ReportKind::Result {
            assert_eq!(child_after.stage, RunStage::Reported);
            assert_eq!(child_after.result.as_ref().unwrap().message_id, message_id);
            assert_eq!(child_after.result.as_ref().unwrap().outcome, Some(ReportOutcome::Succeeded));
        } else {
            assert_eq!(child_after.stage, RunStage::Working);
            assert!(child_after.result.is_none());
        }
        report_ids.push(message_id);
    }
    let response = fixture
        .apply(
            &parent_actor,
            OrchestrationAction::InboxPull { after_seq: 0, limit: 100 },
        )
        .unwrap();
    let OrchestrationActionResult::Inbox { messages, read_through_seq } = response.result else {
        panic!("Expected the parent's actual inbox read");
    };
    assert_parent_receipts();
    for message_id in &report_ids {
        let read = messages.iter().find(|message| &message.message_id == message_id).unwrap();
        assert_agent_decision(read, &child);
        assert_eq!(read.to_run_id, parent);
        assert_eq!(read.stage, DeliveryStage::Read);
        assert!(read.seq <= read_through_seq);
        let state = fixture.state();
        let durable = state.messages.iter().find(|message| &message.message_id == message_id).unwrap();
        assert_eq!(durable.stage, DeliveryStage::Read);
    }
    fixture
        .apply(
            &parent_actor,
            OrchestrationAction::InboxAck { through_seq: read_through_seq },
        )
        .unwrap();
    assert_parent_receipts();
    let state = fixture.state();
    for message_id in report_ids {
        let acked = state.messages.iter().find(|message| message.message_id == message_id).unwrap();
        assert_agent_decision(acked, &child);
        assert_eq!(acked.to_run_id, parent);
        assert_eq!(acked.stage, DeliveryStage::Acked);
        assert!(acked.acked_at.is_some());
    }
}

#[test]
fn nested_reports_and_question_pointers_wake_root_once_without_forwarding_subagent_evidence() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, branch, _) = fixture.working(&root);
    let task = fixture.create_task(&root, "Nested managed worker");
    let run_id = fixture.start(&branch, &task);
    fixture.setup(&run_id);
    let worker = fixture.launch_and_bind(&run_id);
    let initialized = report(ReportKind::Initialized, None, Some("Advisory work plan"));
    let needs_input = report(ReportKind::NeedsInput, None, None);
    let result = fixture.successful_result(&worker);
    for (event, action) in [
        ("initialized", initialized),
        ("needs_input", needs_input),
        ("result", result),
    ] {
        let OrchestrationAction::Report { message_id, .. } = &action else {
            unreachable!()
        };
        fixture.apply(&worker, action.clone()).unwrap();
        fixture.apply(&worker, action.clone()).unwrap();
        let messages = fixture.state().messages;
        let receipt = messages
            .iter()
            .find(|message| message.message_id == *message_id)
            .unwrap();
        assert_eq!(receipt.to_run_id, branch);
        assert_eq!(receipt.kind, MessageKind::Report);
        let pointer_id = format!("manage-{run_id}-{message_id}");
        let pointers: Vec<_> = messages
            .iter()
            .filter(|message| message.message_id == pointer_id)
            .collect();
        assert_eq!(pointers.len(), 1);
        assert_eq!(pointers[0].to_run_id, root);
        assert_eq!(pointers[0].kind, MessageKind::Observation);
        assert!(!pointers[0].stale);
        let pointer: serde_json::Value = serde_json::from_str(&pointers[0].text).unwrap();
        assert_eq!(
            pointer,
            serde_json::json!({
                "event": event, "run_id": run_id, "receipt_message_id": message_id,
            })
        );
        assert!(!pointers[0].text.contains("Evidence from isolated worker"));
    }
    fixture.apply(&worker, OrchestrationAction::SubagentUpdate {
        subagent_id: "internal-evidence".into(), parent_subagent_id: None, role: Some("task".into()),
        label: "Registered child evidence".into(), status: SubagentStatus::Running, summary: None,
    }).unwrap();
    let original = fixture.run(&run_id);
    let subagent = fixture.child_actor(&worker, &run_id, "internal-evidence");
    let mut stale = worker;
    caller_mut(&mut stale).auth.attachment_epoch += 1;
    for (index, actor) in [&subagent, &stale].into_iter().enumerate() {
        let action = report(ReportKind::NeedsInput, None, None);
        let OrchestrationAction::Report { message_id, .. } = &action else {
            unreachable!()
        };
        let pointer_id = format!("manage-{run_id}-{message_id}");
        if index == 1 {
            assert_code(fixture.apply(actor, action), "job_unauthorized");
        } else {
            fixture.apply(actor, action).unwrap();
        }
        assert!(
            fixture
                .state()
                .messages
                .iter()
                .all(|message| message.message_id != pointer_id)
        );
        assert_review_fields_only(&original, &fixture.run(&run_id));
    }
    // Direct-root reports already wake the root; no duplicate Observation.
    let (_, direct, direct_worker) = fixture.working(&root);
    let action = report(ReportKind::NeedsInput, None, None);
    let OrchestrationAction::Report { message_id, .. } = &action else {
        unreachable!()
    };
    let pointer_id = format!("manage-{direct}-{message_id}");
    fixture.apply(&direct_worker, action).unwrap();
    assert!(
        fixture
            .state()
            .messages
            .iter()
            .all(|message| message.message_id != pointer_id)
    );
}

#[test]
fn closing_root_retains_live_descendants_and_closed_root_evidence_inbox() {
    let fixture = Fixture::new();
    let root = fixture.root();
    fixture.launch_and_bind(&root);
    let (task, child, worker) = fixture.working(&root);
    let (_, nested, _) = fixture.working(&child);
    let child_before = fixture.run(&child);
    let nested_before = fixture.run(&nested);
    fixture.operator(OrchestrationAction::CancelRun {
        run_id: root.clone(),
    });
    assert_eq!(fixture.run(&root).stage, RunStage::Closed);
    assert_review_fields_only(&child_before, &fixture.run(&child));
    assert_review_fields_only(&nested_before, &fixture.run(&nested));
    assert!(!fixture.task(&root, &task.task_id).checked);
    let child_run = fixture.run(&child);
    let location = child_run.attachment.as_ref().unwrap().location.as_ref().unwrap();
    let board = {
        let locked = fixture.service.store.lock().unwrap();
        let document = locked.tasks(&root).unwrap();
        let graph = dependencies::DependencyGraph::new(&document.tasks);
        TaskBoard {
            root_id: root.clone(),
            path: fixture
                .service
                .base()
                .join("tasks")
                .join(format!("{root}.md"))
                .to_string_lossy()
                .into_owned(),
            doc_revision: document.doc_revision.clone(),
            unidentified_items: document.unidentified_items,
            diagnostics: Vec::new(),
            tasks: document
                .tasks
                .iter()
                .cloned()
                .map(|task| TaskView {
                    dependencies: graph.evaluate(&task),
                    task,
                    lane: TaskLane::Queued,
                    current_run_id: None,
                })
                .collect(),
        }
    };
    let snapshot = projection::snapshot(
        &OrchestrationSnapshotRequest {
            session_id: SESSION.into(),
            root_id: Some(root.clone()),
        },
        &fixture.state(),
        "fixture-token".into(),
        vec![board],
        Ok(herdr::RuntimeView {
            endpoint_identity: location.endpoint_identity.clone(),
            boot_id: location.boot_id.clone(),
            workspaces: Vec::new(),
            panes: vec![herdr::RuntimePane {
                workspace_id: location.workspace_id.clone(),
                workspace_label: "Retained Space".into(),
                tab_id: location.tab_id.clone(),
                tab_label: "Still working".into(),
                pane_id: location.pane_id.clone(),
                terminal_id: location.terminal_id.clone(),
                native_session_id: location.native_session_id.clone(),
                agent_name: Some(location.launch_tag.clone()),
                agent_kind: Some("omp".into()),
                launch_pending: false,
                interactive_ready: false,
                agent_status: Some("working".into()),
                state_changed_at: None,
            }],
        }),
        &now(),
        "fixture-store",
    )
    .unwrap();
    let current_task = snapshot
        .board
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|view| view.task.task_id == task.task_id)
        .unwrap();
    assert_eq!(current_task.lane, TaskLane::Working);
    assert_eq!(current_task.current_run_id.as_deref(), Some(child.as_str()));
    assert_eq!(snapshot.roots[0].open_runs, 2);
    assert_eq!(
        snapshot
            .runs
            .iter()
            .find(|run| run.run_id == child)
            .unwrap()
            .stage,
        RunStage::Working
    );
    let RuntimeObservation::Fresh { runs, .. } = snapshot.runtime else {
        panic!("Expected fresh observation")
    };
    let observed = runs.iter().find(|run| run.run_id == child).unwrap();
    assert_eq!(observed.presence, Presence::Present);
    assert_eq!(observed.agent_status.as_deref(), Some("working"));
    assert!(
        observed.actual_omp,
        "Closing tracking is not process termination"
    );
    fixture.apply(&worker, fixture.successful_result(&worker))
        .unwrap();
    assert_eq!(fixture.run(&child).stage, RunStage::Reported);
    let receipt = fixture.run(&child).result.unwrap().message_id;
    assert!(fixture.state().messages.iter().any(|message| {
        message.message_id == receipt && message.to_run_id == root && !message.stale
    }));
    assert!(!fixture.task(&root, &task.task_id).checked);
}

#[test]
fn public_task_assignment_is_idempotent_operator_only_and_resolves_current_task_by_exact_revision()
{
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task_id = id();
    let action = OrchestrationAction::TaskAssign {
        root_id: root.clone(),
        task_id: task_id.clone(),
        title: "Assigned canonical task".into(),
        description: "Canonical body is never copied to inbox".into(),
    };
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(&supervisor, action.clone()),
        "actor_forbidden",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::TaskAssigned {
        task,
        to_run_id,
        seq,
        duplicate,
    } = first.result
    else {
        panic!("Expected durable assignment");
    };
    assert_eq!(task.task_id, task_id);
    assert_eq!(to_run_id, root);
    assert!(!duplicate);
    let repeated = fixture.operator(action);
    let OrchestrationActionResult::TaskAssigned {
        seq: repeated_seq,
        duplicate,
        ..
    } = repeated.result
    else {
        panic!("Expected same durable assignment");
    };
    assert!(duplicate);
    assert_eq!(repeated_seq, seq);
    let messages = fixture.state().messages;
    let assigned: Vec<_> = messages
        .iter()
        .filter(|message| message.message_id == format!("assign-{task_id}"))
        .collect();
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].kind, MessageKind::Instruction);
    assert!(matches!(assigned[0].from, ActorRef::Operator));
    let pointer: serde_json::Value = serde_json::from_str(&assigned[0].text).unwrap();
    assert_eq!(pointer["event"], "task_assigned");
    assert_eq!(pointer["task_id"], task_id);
    assert_eq!(pointer["task_revision"], task.task_revision);
    assert_eq!(pointer["origin"], "browser");
    assert!(pointer.get("title").is_none());
    assert!(pointer.get("body").is_none());
    assert!(fixture.state().assignment_intents.is_empty());
    let conflict_id = id();
    let existing = {
        let locked = fixture.service.store.lock().unwrap();
        locked
            .tasks(&root)
            .unwrap()
            .create_with_id(
                &conflict_id,
                "Authoritative external title",
                "Authoritative external body",
            )
            .unwrap()
    };
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Native),
            OrchestrationAction::TaskAssign {
                root_id: root.clone(),
                task_id: conflict_id.clone(),
                title: "Submitted title".into(),
                description: "Submitted body".into(),
            },
        ),
        "task_assignment_conflict",
    );
    assert_eq!(fixture.state().assignment_intents.len(), 1);
    assert_eq!(
        fixture.state().assignment_intents[0].state,
        IntentState::Conflict
    );
    assert!(
        fixture
            .state()
            .messages
            .iter()
            .all(|message| message.message_id != format!("assign-{conflict_id}"))
    );
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::TaskAssignmentResolve {
                root_id: root.clone(),
                task_id: conflict_id.clone(),
                expected_task_revision: Some("unreviewed-revision".into()),
                assign: true,
            },
        ),
        "task_revision_conflict",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    assert_code(
        fixture.apply(
            &supervisor,
            OrchestrationAction::TaskAssignmentResolve {
                root_id: root.clone(),
                task_id: conflict_id.clone(),
                expected_task_revision: Some(existing.task_revision.clone()),
                assign: true,
            },
        ),
        "actor_forbidden",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    let resolved = fixture.operator(OrchestrationAction::TaskAssignmentResolve {
        root_id: root.clone(),
        task_id: conflict_id.clone(),
        expected_task_revision: Some(existing.task_revision.clone()),
        assign: true,
    });
    let OrchestrationActionResult::TaskAssigned { task: current, .. } = resolved.result else {
        panic!("Expected current task assigned")
    };
    assert_eq!(current.title, existing.title);
    assert_eq!(current.body, existing.body);
    assert_eq!(current.task_revision, existing.task_revision);
    assert!(fixture.state().assignment_intents.is_empty());
    let messages = fixture.state().messages;
    let assignment = messages
        .iter()
        .find(|message| message.message_id == format!("assign-{conflict_id}"))
        .unwrap();
    let pointer: serde_json::Value = serde_json::from_str(&assignment.text).unwrap();
    assert_eq!(
        pointer["origin"], "native",
        "Resolution retains actual assignment provenance"
    );
}

#[test]
fn supervisor_worker_start_uses_canonical_fences_and_replays_before_machine_cas() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "CAS worker start");
    let action = fixture.start_action(&root, &task);
    let reviewed_revision = fixture.state().revision;
    fixture.operator(OrchestrationAction::Annotate {
        run_id: root.clone(), text: "Concurrent unrelated machine history".into(),
    });
    let request = OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: Some(reviewed_revision), action,
    };
    let first = fixture.service.mutate(&supervisor, request.clone()).unwrap();
    let OrchestrationActionResult::WorkerStarted { start } = first.result else { unreachable!() };
    assert!(!start.duplicate);
    assert_eq!(fixture.run(&start.run_id).parent_run_id.as_deref(), Some(root.as_str()));
    let bytes = fixture.state_bytes();
    let replay = fixture.service.mutate(&supervisor, request).unwrap();
    let OrchestrationActionResult::WorkerStarted { start: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.run_id, start.run_id);
    assert_eq!(fixture.state_bytes(), bytes);
}


#[test]
fn abandoning_conflicted_assignment_preserves_authoritative_markdown_even_after_root_closure() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task_id = id();
    {
        let locked = fixture.service.store.lock().unwrap();
        locked
            .tasks(&root)
            .unwrap()
            .create_with_id(
                &task_id,
                "Keep this existing task",
                "External content survives abandonment",
            )
            .unwrap();
    }
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::TaskAssign {
                root_id: root.clone(),
                task_id: task_id.clone(),
                title: "Different submitted draft".into(),
                description: "Do not overwrite authoritative task".into(),
            },
        ),
        "task_assignment_conflict",
    );
    fixture.operator(OrchestrationAction::CancelRun {
        run_id: root.clone(),
    });
    let markdown_path = fixture
        .service
        .base()
        .join("tasks")
        .join(format!("{root}.md"));
    let before = std::fs::read(&markdown_path).unwrap();
    let result = fixture.operator(OrchestrationAction::TaskAssignmentResolve {
        root_id: root.clone(),
        task_id: task_id.clone(),
        expected_task_revision: None,
        assign: false,
    });
    assert!(matches!(result.result, OrchestrationActionResult::Done));
    assert!(fixture.state().assignment_intents.is_empty());
    assert_eq!(std::fs::read(markdown_path).unwrap(), before);
    assert!(
        fixture
            .state()
            .messages
            .iter()
            .all(|message| message.message_id != format!("assign-{task_id}"))
    );
    assert_eq!(
        fixture.task(&root, &task_id).title,
        "Keep this existing task"
    );
}

#[test]
fn acceptance_intent_recovery_cannot_accept_cancelled_restarted_failed_or_other_task_runs() {
    for race in 0..7 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (task, run_id, actor) = fixture.working(&root);
        fixture.apply(&actor, fixture.successful_result(&actor))
            .unwrap();
        let intent_id = fixture.pending_intent(&root, &task, &run_id);
        fixture.seed_run(&run_id, |run| match race {
            0 => {
                run.stage = RunStage::Closed;
                run.close_reason = Some(CloseReason::Cancelled);
            }
            1 => {
                run.stage = RunStage::Starting;
                run.attachment.as_mut().unwrap().main_session_id = None;
                run.dispatch.as_mut().unwrap().launch_attempt += 1;
            }
            2 => run.result.as_mut().unwrap().outcome = Some(ReportOutcome::Failed),
            3 => run.result = None,
            4 => run.task_id = Some(id()),
            5 => run.root_id = id(),
            6 => run.stage = RunStage::Working,
            _ => unreachable!(),
        });
        let before = fixture.run(&run_id);
        let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
        reopened.recover_intents().unwrap();
        assert_review_fields_only(&before, &fixture.run(&run_id));
        assert!(!fixture.task(&root, &task.task_id).checked);
        let state = fixture.state();
        assert_eq!(state.task_intents.len(), 1);
        assert_eq!(state.task_intents[0].intent_id, intent_id);
        assert_eq!(state.task_intents[0].state, IntentState::Conflict);
    }
}

fn dispatch_events(fixture: &Fixture, root: &str, event: &str) -> Vec<Message> {
    fixture.state().messages.into_iter().filter(|message| {
        message.to_run_id == root && matches!(message.from, ActorRef::Dispatcher)
            && serde_json::from_str::<serde_json::Value>(&message.text)
                .is_ok_and(|body| body["event"] == event)
    }).collect()
}

#[test]
fn dispatch_failure_wakes_root_once_with_typed_next_steps() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "Recover planning");
    let worker = fixture.start(&root, &task);
    let failure = || DispatchUpdate::PlanFailed { error: ErrorResponse {
        code: "endpoint_unavailable".into(), message: "Herdr endpoint is unavailable".into(),
    }};
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    let events = dispatch_events(&fixture, &root, "dispatch_failure");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, MessageKind::Observation);
    assert_eq!(events[0].stage, DeliveryStage::Stored);
    let run = fixture.run(&worker);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&events[0].text).unwrap(), serde_json::json!({
        "event": "dispatch_failure", "run_id": worker, "task_id": task.task_id,
        "run_attempt": run.attempt, "launch_attempt": run.dispatch.as_ref().unwrap().launch_attempt,
        "stage": run.stage, "step": "plan_failed", "agent_started": false,
        "error": {"code": "endpoint_unavailable", "message": "Herdr endpoint is unavailable"},
        "automatic": "none", "effect": "none", "next": ["show", "reconcile"], "operator_reason": null,
    }));
    fixture.apply(&supervisor, OrchestrationAction::InboxWoken {
        omp_session_id: caller_mut(&mut supervisor.clone()).omp_session_id.clone().unwrap(),
        through_seq: events[0].seq,
    }).unwrap();
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 1);
    fixture.apply(&supervisor, OrchestrationAction::InboxPull { after_seq: 0, limit: 100 }).unwrap();
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 2);
    fixture.service.record_dispatch(&root, failure()).unwrap();
    assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 2);
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    let reopened_state = {
        let locked = reopened.store.lock().unwrap();
        locked.read().unwrap()
    };
    assert_eq!(reopened_state.revision, fixture.state().revision);
    assert_review_fields_only(&fixture.run(&worker),
        &reopened_state.runs[run_index(&reopened_state, &worker).unwrap()]);
    assert_eq!(serde_json::to_value(reopened_state.messages).unwrap(),
        serde_json::to_value(fixture.state().messages).unwrap());
}

#[test]
fn launch_review_failure_classifies_effect_and_operator_cases() {
    for case in 0..7 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Classified recovery");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        if matches!(case, 0 | 1 | 5 | 6) {
            fixture.launch_and_bind(&worker);
        }
        fixture.seed_run(&worker, |run| {
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.step = DispatchStep::LaunchIntent;
            if matches!(case, 2 | 3) {
                let setup = run.setup.as_mut().unwrap();
                setup.operation_id = Some("uncertain-operation".into());
                setup.workspace_id = None;
            }
            if case == 1 { run.attachment.as_mut().unwrap().process = None; }
            if case == 5 { run.stage = RunStage::Reported; }
            if case == 6 { run.stage = RunStage::Closed; }
        });
        if case == 6 {
            let bytes = fixture.state_bytes();
            fixture.service.record_launch_review(&fixture.run(&worker), DispatchStep::NeedsReview, Some(review_error())).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
        } else if matches!(case, 2 | 3) {
            fixture.service.record_dispatch(&worker, DispatchUpdate::Step {
                step: DispatchStep::SetupUnknown,
                error: Some(ErrorResponse { code: if case == 2 { "workspace_conflict" } else { "outcome_unknown" }.into(),
                    message: "Checkout effect is uncertain".into() }),
            }).unwrap();
        } else {
            fixture.service.record_launch_review(&fixture.run(&worker), DispatchStep::NeedsReview,
                Some(ErrorResponse { code: "launch_identity_conflict".into(), message: "é".repeat(1000) })).unwrap();
        }
        let events = dispatch_events(&fixture, &root, "dispatch_failure");
        if case >= 5 {
            assert!(events.is_empty());
            continue;
        }
        assert_eq!(events.len(), 1);
        let body: serde_json::Value = serde_json::from_str(&events[0].text).unwrap();
        let effect = if case == 4 { "none" } else { "unknown" };
        assert_eq!(body["effect"], effect);
        assert_eq!(body["operator_reason"].is_string(), matches!(case, 1..=3));
        assert!(body["error"]["message"].as_str().unwrap().len() <= 1024);
    }
}

fn observed_root_pane(fixture: &Fixture, root: &str) -> herdr::RuntimePane {
    let location = fixture.run(root).attachment.unwrap().location.unwrap();
    herdr::RuntimePane {
        workspace_id: location.workspace_id,
        workspace_label: "Project".into(),
        tab_id: location.tab_id,
        tab_label: "Supervisor".into(),
        pane_id: location.pane_id,
        terminal_id: location.terminal_id,
        native_session_id: location.native_session_id,
        agent_name: None,
        agent_kind: Some("omp".into()),
        launch_pending: false,
        interactive_ready: true,
        agent_status: Some("working".into()),
        state_changed_at: None,
    }
}

#[test]
fn unstarted_unknown_launch_escalates_after_settle_and_recovers_once() {
    for exhausted in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Automatic proof before escalation");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        fixture.service.record_dispatch(&worker, DispatchUpdate::LaunchIntent {
            launch_tag: format!("launch-{worker}-1"), launch_attempt: 1, endpoint_identity: "endpoint-one".into(),
        }).unwrap();
        fixture.service.record_launch_review(&fixture.run(&worker), DispatchStep::LaunchUnknown, Some(ErrorResponse {
            code: "omp_start_unobserved".into(), message: "OMP start was not observed before its deadline".into(),
        })).unwrap();
        assert!(dispatch_events(&fixture, &root, "dispatch_failure").is_empty());
        let runtime = herdr::RuntimeView {
            endpoint_identity: "endpoint-one".into(), boot_id: Some("boot-one".into()),
            workspaces: Vec::new(), panes: vec![observed_root_pane(&fixture, &root)],
        };
        let bytes = fixture.state_bytes();
        let receiver = fixture.service.subscribe();
        fixture.service.record_observation(&runtime, SESSION).unwrap();
        assert_eq!(fixture.state_bytes(), bytes);
        assert!(!receiver.has_changed().unwrap());
        if exhausted {
            fixture.seed_run(&worker, |run| {
                run.dispatch.as_mut().unwrap().updated_at =
                    (time::OffsetDateTime::now_utc() - time::Duration::milliseconds(DEFAULT_START_TIMEOUT_MS as i64 + 1000))
                        .format(&time::format_description::well_known::Rfc3339).unwrap();
            });
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            let events = dispatch_events(&fixture, &root, "dispatch_failure");
            assert_eq!(events.len(), 1);
            assert!(events[0].message_id.ends_with(":launch_unresolved"));
            let body: serde_json::Value = serde_json::from_str(&events[0].text).unwrap();
            assert_eq!(body["automatic"], "exhausted");
            assert_eq!(body["effect"], "unknown");
            let bytes = fixture.state_bytes();
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
            fixture.seed_run(&worker, |run| {
                run.dispatch.as_mut().unwrap().error = Some(ErrorResponse { code: "launch_outcome_unknown".into(), message: "Different evidence, same launch".into() });
            });
            let bytes = fixture.state_bytes();
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
            assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 1);
        }
        // A late actual-main bind and proof resolves this same launch incarnation.
        let actor = fixture.launch_tab(&worker);
        let Actor::Agent(caller) = &actor else { unreachable!() };
        fixture.apply(&actor, OrchestrationAction::RunAttach { operation_id: id(), attachment_id: caller.auth.attachment_id.clone(), expected_attachment_epoch: caller.auth.attachment_epoch, main_session_id: caller.omp_session_id.clone().unwrap() }).unwrap();
        let reviewed = fixture.run(&worker);
        fixture.service.record_launch_verified(&reviewed).unwrap();
        let notices = dispatch_events(&fixture, &root, "dispatch_recovered");
        assert_eq!(notices.len(), usize::from(exhausted));
        let bytes = fixture.state_bytes();
        fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
        assert_eq!(fixture.state_bytes(), bytes);
        assert_eq!(dispatch_events(&fixture, &root, "dispatch_recovered").len(), usize::from(exhausted));
    }
}

#[test]
fn supervisor_recovery_preserves_machine_cas_and_automatic_launch_ownership() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "Uncertain setup recovery");
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    fixture.service.record_dispatch(&worker, DispatchUpdate::Step {
        step: DispatchStep::SetupUnknown, error: Some(review_error()),
    }).unwrap();
    let revision = fixture.state().revision;
    fixture.operator(OrchestrationAction::Annotate { run_id: root.clone(), text: "Unrelated newer history".into() });
    for action in [
        OrchestrationAction::ReconcileRun { run_id: worker.clone(), recovery: None },
    ] {
        let request = OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: Some(revision), action };
        let bytes = fixture.state_bytes();
        let result = fixture.service.mutate(&supervisor, request);
        assert_code(result, "orchestration_revision_conflict");
        assert_eq!(fixture.state_bytes(), bytes);
    }
    fixture.seed_run(&worker, |run| {
        run.dispatch.as_mut().unwrap().step = DispatchStep::SetupUnknown;
        run.dispatch.as_mut().unwrap().launch_tag = None;
        run.setup.as_mut().unwrap().operation_id = Some("checkout-recovery-operation".into());
        run.setup.as_mut().unwrap().workspace_id = None;
    });
    fixture.apply(&supervisor, OrchestrationAction::ReconcileRun {
        run_id: worker.clone(),
        recovery: Some(cockpit_protocol::projects::WorkspaceRecoveryAction::AcceptExistingWorktree),
    }).unwrap();
    let recovery = fixture.run(&worker);
    assert_eq!(recovery.dispatch.as_ref().unwrap().step, DispatchStep::SetupUnknown);
    assert_eq!(recovery.dispatch.as_ref().unwrap().recovery,
        Some(cockpit_protocol::projects::WorkspaceRecoveryAction::AcceptExistingWorktree));
}

#[test]
fn old_unknown_launch_with_fresh_bound_omp_proof_stays_quiet_before_reconcile() {
    for evidence in 0..5 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Recover an old launch outcome");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        fixture.launch_and_bind(&worker);
        fixture.seed_run(&worker, |run| {
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.step = DispatchStep::LaunchUnknown;
            dispatch.agent_started = false;
            dispatch.error = Some(ErrorResponse {
                code: "omp_start_unobserved".into(), message: "Prior installation did not recognize this bound launch".into(),
            });
            dispatch.updated_at = (time::OffsetDateTime::now_utc() - time::Duration::hours(2))
                .format(&time::format_description::well_known::Rfc3339).unwrap();
        });
        let location = fixture.run(&worker).attachment.unwrap().location.unwrap();
        let runtime = herdr::RuntimeView {
            endpoint_identity: location.endpoint_identity.clone(), boot_id: location.boot_id.clone(),
            workspaces: Vec::new(),
            panes: vec![herdr::RuntimePane {
                workspace_id: location.workspace_id.clone(), workspace_label: "Project".into(),
                tab_id: location.tab_id.clone(), tab_label: "Worker".into(), pane_id: location.pane_id.clone(),
                terminal_id: location.terminal_id.clone(),
                native_session_id: if evidence == 4 { Some("replacement-native-session".into()) } else { location.native_session_id.clone() },
                agent_name: if evidence == 1 { Some("changed-display-name".into()) } else { None },
                agent_kind: if evidence == 3 { None } else { Some("omp".into()) },
                launch_pending: evidence == 2, interactive_ready: false,
                agent_status: Some("working".into()), state_changed_at: None,
            }, observed_root_pane(&fixture, &root)],
        };
        let bytes = fixture.state_bytes();
        fixture.service.record_observation(&runtime, SESSION).unwrap();
        let failures = dispatch_events(&fixture, &root, "dispatch_failure");
        if evidence <= 1 {
            assert!(failures.is_empty());
            assert_eq!(fixture.state_bytes(), bytes);
            fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
            assert_eq!(fixture.run(&worker).dispatch.as_ref().unwrap().step, DispatchStep::Launched);
            assert!(dispatch_events(&fixture, &root, "dispatch_recovered").is_empty());
        } else {
            assert_eq!(failures.len(), 1);
            let bytes = fixture.state_bytes();
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
        }
    }
}

fn task_document_revision(fixture: &Fixture, root: &str) -> String {
    fixture.service.store.lock().unwrap().tasks(root).unwrap().doc_revision.clone()
}

fn canonical_check(fixture: &Fixture, root: &str, task_id: &str, checked: bool) {
    let locked = fixture.service.store.lock().unwrap();
    let document = locked.tasks(root).unwrap();
    let revision = document.task(task_id).unwrap().task_revision.clone();
    document.check(task_id, &revision, checked).unwrap();
}

fn related_task(fixture: &Fixture, root: &str, edges: &[String], follow: Option<&Task>) -> Task {
    task_result(fixture.operator(OrchestrationAction::TaskCreate {
        root_id: root.into(), task_id: id(), title: "Related task".into(),
        description: "Protected description".into(), depends_on: edges.to_vec(),
        follow_up_of: follow.map(|task| task.task_id.clone()),
        expected_doc_revision: Some(task_document_revision(fixture, root)),
        source_revision: follow.map(|task| task.task_revision.clone()),
    }))
}

fn execute_task(fixture: &Fixture, root: &str, task: &Task) -> (String, Actor) {
    let run_id = fixture.start(root, task);
    fixture.setup(&run_id);
    let actor = fixture.launch_and_bind(&run_id);
    fixture.initialized(&actor);
    (run_id, actor)
}

fn content_actions(root: &str, task: &Task) -> Vec<OrchestrationAction> {
    let step_id = id();
    vec![
        OrchestrationAction::TaskUpdate {
            root_id: root.into(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
            title: Some("Unauthorized title".into()), description: Some("Unauthorized prose".into()),
        },
        OrchestrationAction::TaskStepAdd {
            root_id: root.into(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
            step_id: step_id.clone(), parent_step_id: None, before_step_id: None, title: "Unauthorized step".into(),
        },
        OrchestrationAction::TaskStepRename {
            root_id: root.into(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
            step_id: step_id.clone(), title: "Unauthorized rename".into(),
        },
        OrchestrationAction::TaskStepSetChecked {
            root_id: root.into(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
            step_id: step_id.clone(), checked: true, scope: TaskStepScope::Leaf,
        },
        OrchestrationAction::TaskStepMove {
            root_id: root.into(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
            step_id: step_id.clone(), parent_step_id: None, before_step_id: None,
        },
        OrchestrationAction::TaskStepRemove {
            root_id: root.into(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(), step_id,
        },
    ]
}

#[test]
fn all_content_actions_require_the_current_executed_task_owner_not_generic_root_scope() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, worker, actor) = fixture.working(&root);
    let sibling = fixture.create_task(&root, "Sibling canonical task");
    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let root_actor = fixture.launch_and_bind(&root);
    let operator = Actor::Operator(OperatorOrigin::Browser);
    for manager in [&root_actor, &operator] {
        for action in content_actions(&root, &task) {
            let bytes = std::fs::read(&path).unwrap();
            assert_code(fixture.apply(manager, action), "actor_forbidden");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    for action in content_actions(&root, &sibling) {
        let bytes = std::fs::read(&path).unwrap();
        assert_code(fixture.apply(&actor, action), "actor_forbidden");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    let own = task_result(fixture.apply(&actor, OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
        title: None, description: Some("Executed own-task prose".into()),
    }).unwrap());
    assert_eq!(own.description, "Executed own-task prose");
    let original = fixture.run(&worker);
    for missing in 0..3 {
        fixture.seed_run(&worker, |run| {
            *run = original.clone();
            match missing {
                0 => run.checkout_claim_id = None,
                1 => run.task_id = Some(sibling.task_id.clone()),
                _ => run.attachment.as_mut().unwrap().epoch += 1,
            }
        });
        let bytes = std::fs::read(&path).unwrap();
        assert!(fixture.apply(&actor, content_actions(&root, &own).remove(0)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    for stage in [RunStage::Starting, RunStage::Reported, RunStage::Closed] {
        fixture.seed_run(&worker, |run| { *run = original.clone(); run.stage = stage; });
        let bytes = std::fs::read(&path).unwrap();
        assert_code(fixture.apply(&actor, content_actions(&root, &own).remove(0)),
            if stage == RunStage::Closed { "job_unauthorized" } else { "actor_forbidden" });
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    fixture.seed_run(&worker, |run| { *run = original; run.stage = RunStage::Reported; });
    // The deliberate root/operator edit of unchecked Reported work remains permitted.
    let edited = task_result(fixture.apply(&root_actor, OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: own.task_id.clone(), expected_task_revision: own.task_revision,
        title: Some("Root-reviewed correction".into()), description: None,
    }).unwrap());
    assert_eq!(edited.title, "Root-reviewed correction");
    let edited = task_result(fixture.apply(&operator, OrchestrationAction::TaskStepAdd {
        root_id: root.clone(), task_id: edited.task_id.clone(), expected_task_revision: edited.task_revision,
        step_id: id(), parent_step_id: None, before_step_id: None, title: "Operator-reviewed remaining step".into(),
    }).unwrap());
    assert_eq!(edited.steps[0].title, "Operator-reviewed remaining step");
    canonical_check(&fixture, &root, &edited.task_id, true);
    for action in content_actions(&root, &fixture.task(&root, &edited.task_id)) {
        assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), action), "task_checked");
    }
}

#[test]
fn child_content_authority_requires_scoped_credentials_and_current_running_subject() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, worker, parent) = fixture.working(&root);
    let step_id = id();
    let task = task_result(fixture.apply(&parent, OrchestrationAction::TaskStepAdd {
        root_id: root.clone(), task_id: task.task_id, expected_task_revision: task.task_revision,
        step_id: step_id.clone(), parent_step_id: None, before_step_id: None, title: "Native child leaf".into(),
    }).unwrap());
    let update = OrchestrationAction::SubagentUpdate {
        subagent_id: "actual-child".into(), parent_subagent_id: None, role: Some("task".into()),
        label: "Parent telemetry is not a child credential".into(), status: SubagentStatus::Running, summary: None,
    };
    let check = |task: &Task| OrchestrationAction::TaskStepSetChecked {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
        step_id: step_id.clone(), checked: true, scope: TaskStepScope::Leaf,
    };
    let mut child = fixture.child_actor(&parent, &worker, "actual-child");
    assert_code(fixture.apply(&child, check(&task)), "actor_forbidden");
    fixture.apply(&parent, update.clone()).unwrap();
    assert!(fixture.state().subagents[0].bound_omp_session.is_none());
    // A credential authorizes its subject even offline; optional physical context
    // neither narrows ordinary admission nor impersonates another registry child.
    let caller = caller_mut(&mut child);
    caller.provenance = None;
    caller.omp_session_id = None;
    caller.process = None;
    caller.agent_kind = Some(AgentKind::Main);
    caller.subagent_id = Some("not-the-credential-subject".into());
    let checked = task_result(fixture.apply(&child, check(&task)).unwrap());
    assert_eq!(checked.step_progress.as_ref().map(|progress| (progress.done, progress.total)), Some((1, 1)));
    assert!(!checked.checked, "Step completion is not task acceptance");
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
    let sibling = fixture.create_task(&root, "Not the child's task");
    assert_code(fixture.apply(&child, content_actions(&root, &sibling).remove(0)), "actor_forbidden");
    let unregistered = fixture.child_actor(&parent, &worker, "unregistered-child");
    assert_code(fixture.apply(&unregistered, check(&checked)), "actor_forbidden");
    let mut stale = child.clone();
    caller_mut(&mut stale).auth.attachment_epoch += 1;
    assert_code(fixture.apply(&stale, check(&checked)), "job_unauthorized");
    let bytes = fixture.state_bytes();
    assert!(fixture.apply(&child, fixture.successful_result(&child)).is_err());
    assert_eq!(fixture.state_bytes(), bytes, "A child cannot file its worker's Result");
    let mut done = update;
    if let OrchestrationAction::SubagentUpdate { status, .. } = &mut done {
        *status = SubagentStatus::Done;
    }
    fixture.apply(&parent, done).unwrap();
    assert_code(fixture.apply(&child, check(&checked)), "actor_forbidden");
}

#[test]
fn blocked_dependencies_never_replace_the_current_live_worker() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Prerequisite");
    canonical_check(&fixture, &root, &source.task_id, true);
    let task = related_task(&fixture, &root, &[source.task_id.clone()], None);
    let (old, _) = execute_task(&fixture, &root, &task);
    canonical_check(&fixture, &root, &source.task_id, false);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        fixture.start_action(&root, &task)), "task_has_active_run");
    assert_eq!(fixture.state_bytes(), bytes);
    assert_eq!(fixture.run(&old).stage, RunStage::Working);
    assert_eq!(projection::current_task_run(&fixture.state(), &root, &task.task_id).unwrap().run_id, old);
    assert!(!fixture.service.dispatch_task_eligible(&fixture.run(&old)).unwrap());
}

#[test]
fn dependency_changes_outside_target_bytes_gate_content_accept_sendback_and_dispatch() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Prerequisite");
    canonical_check(&fixture, &root, &source.task_id, true);
    let task = related_task(&fixture, &root, &[source.task_id.clone()], None);
    let run_id = fixture.start(&root, &task);
    fixture.setup(&run_id);
    let actor = fixture.launch_and_bind(&run_id);
    fixture.initialized(&actor);
    canonical_check(&fixture, &root, &source.task_id, false);
    assert_eq!(fixture.task(&root, &task.task_id).task_revision, task.task_revision);
    assert!(!fixture.service.dispatch_task_eligible(&fixture.run(&run_id)).unwrap());
    // No global task freeze: independent work remains runnable.
    let independent = fixture.create_task(&root, "Independent task");
    let (independent_run, _) = execute_task(&fixture, &root, &independent);
    assert!(fixture.service.dispatch_task_eligible(&fixture.run(&independent_run)).unwrap());
    canonical_check(&fixture, &root, &source.task_id, false);
    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let before = std::fs::read(&path).unwrap();
    for action in content_actions(&root, &task) {
        assert_code(fixture.apply(&actor, action), "task_blocked");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    canonical_check(&fixture, &root, &source.task_id, true);
    fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
    assert!(!fixture.task(&root, &task.task_id).checked);
    canonical_check(&fixture, &root, &source.task_id, false);
    let state = fixture.state_bytes();
    for action in [
        OrchestrationAction::Accept { run_id: run_id.clone(), expected_task_revision: task.task_revision.clone() },
        OrchestrationAction::SendBack { run_id: run_id.clone(), text: "Fresh review".into() },
    ] {
        assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), action), "task_blocked");
        assert_eq!(fixture.state_bytes(), state);
        assert!(fixture.state().task_intents.is_empty());
    }
}

#[test]
fn acceptance_recovery_rechecks_unpublished_prerequisites_but_never_rolls_back_published_checkbox() {
    for published in [false, true] {
        for damage in 0..3 {
            let fixture = Fixture::new();
            let root = fixture.root();
            let source = fixture.create_task(&root, "Prerequisite");
            canonical_check(&fixture, &root, &source.task_id, true);
            let task = related_task(&fixture, &root, &[source.task_id.clone()], None);
            let (run_id, actor) = execute_task(&fixture, &root, &task);
            fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
            fixture.pending_intent(&root, &task, &run_id);
            if published { canonical_check(&fixture, &root, &task.task_id, true); }
            let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
            match damage {
                0 => canonical_check(&fixture, &root, &source.task_id, false),
                1 => {
                    let raw = std::fs::read_to_string(&path).unwrap();
                    // Deleting only the prerequisite leaves target full-item CAS unchanged.
                    let start = raw.find(&format!("- [x] Prerequisite <!-- cockpit-task: {} -->", source.task_id)).unwrap();
                    let next = raw[start + 1..].find("\n- [").map(|offset| start + 1 + offset + 1).unwrap();
                    let mut changed = raw.clone();
                    changed.replace_range(start..next, "");
                    std::fs::write(&path, changed).unwrap();
                }
                _ => {
                    let locked = fixture.service.store.lock().unwrap();
                    let document = locked.tasks(&root).unwrap();
                    let source = document.task(&source.task_id).unwrap();
                    document.set_dependencies(&source.task_id, &source.task_revision, &document.doc_revision,
                        &[task.task_id.clone()]).unwrap();
                }
            }
            let target_revision = fixture.task(&root, &task.task_id).task_revision;
            let bytes = std::fs::read(&path).unwrap();
            fixture.service.recover_intents().unwrap();
            if published {
                assert_eq!(fixture.run(&run_id).close_reason, Some(CloseReason::Accepted));
                assert!(fixture.state().task_intents.is_empty());
                assert!(fixture.task(&root, &task.task_id).checked);
            } else {
                assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
                assert_eq!(fixture.state().task_intents[0].state, IntentState::Conflict);
                assert!(!fixture.task(&root, &task.task_id).checked);
                assert!(fixture.run(&run_id).annotations.iter().any(|entry| entry.text.contains("Acceptance recovery retained conflict")));
                assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::IntentResolve {
                    intent_id: fixture.state().task_intents[0].intent_id.clone(), apply: true,
                }), if damage == 0 { "task_blocked" } else { "task_dependencies_invalid" });
            }
            assert_eq!(fixture.task(&root, &task.task_id).task_revision, target_revision);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}

#[test]
fn live_relationship_removal_is_strict_root_only_and_preserves_other_owned_slots() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Source");
    let extra = fixture.create_task(&root, "Extra prerequisite");
    let task = related_task(&fixture, &root, &[source.task_id.clone(), extra.task_id.clone()], Some(&source));
    let task = task_result(fixture.operator(OrchestrationAction::TaskStepAdd {
        root_id: root.clone(), task_id: task.task_id, expected_task_revision: task.task_revision,
        step_id: id(), parent_step_id: None, before_step_id: None, title: "Preserved step".into(),
    }));
    fixture.start(&root, &task); // Queued work already owns its canonical task.
    let worker_task = fixture.create_task(&root, "Independent worker");
    let (_, worker) = execute_task(&fixture, &root, &worker_task);
    let action = |edges: Vec<String>| OrchestrationAction::TaskDependenciesSet {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
        expected_doc_revision: task_document_revision(&fixture, &root), depends_on: edges,
    };
    assert_code(fixture.apply(&worker, action(vec![source.task_id.clone()])), "actor_forbidden");
    let unchanged = task_result(fixture.operator(action(vec![source.task_id.clone(), extra.task_id.clone()])));
    assert_eq!(unchanged.task_revision, task.task_revision);
    assert!(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        action(vec![source.task_id.clone(), worker_task.task_id.clone()])).is_err());
    let before_state = fixture.state_bytes();
    let removed = task_result(fixture.operator(action(vec![source.task_id.clone()])));
    assert_eq!(removed.description, task.description);
    assert_eq!(&removed.body[removed.body.find("<!-- cockpit-checklist: begin -->").unwrap()..],
        &task.body[task.body.find("<!-- cockpit-checklist: begin -->").unwrap()..]);
    assert_eq!(removed.follow_up_of, task.follow_up_of);
    assert_eq!(fixture.state_bytes(), before_state, "Relationship removal never grants/cancels/restarts work");
    let locked = fixture.service.store.lock().unwrap();
    let document = locked.tasks(&root).unwrap();
    assert_eq!(dependencies::DependencyGraph::new(&document.tasks).evaluate(document.task(&task.task_id).unwrap()).state,
        TaskDependencyState::Blocked);
}

#[test]
fn relationship_doc_cas_fences_other_task_bytes_and_create_replay_bypasses_only_stale_values() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Source");
    let task_id = id();
    let action = OrchestrationAction::TaskCreate {
        root_id: root.clone(), task_id: task_id.clone(), title: "Stable follow-up".into(),
        description: "Retained UUID".into(), depends_on: vec![source.task_id.clone()], follow_up_of: Some(source.task_id.clone()),
        expected_doc_revision: Some(task_document_revision(&fixture, &root)), source_revision: Some(source.task_revision.clone()),
    };
    let created = task_result(fixture.operator(action.clone()));
    let old_doc = task_document_revision(&fixture, &root);
    fixture.operator(OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: source.task_id.clone(), expected_task_revision: source.task_revision,
        title: Some("Changed source".into()), description: None,
    });
    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(serde_json::to_value(task_result(fixture.operator(action.clone()))).unwrap(),
        serde_json::to_value(&created).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut missing_fence = action.clone();
    if let OrchestrationAction::TaskCreate { expected_doc_revision, .. } = &mut missing_fence {
        *expected_doc_revision = None;
    }
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), missing_fence), "invalid_task");
    let mut new_identity = action;
    if let OrchestrationAction::TaskCreate { task_id, .. } = &mut new_identity { *task_id = id(); }
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), new_identity), "task_revision_conflict");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::TaskDependenciesSet {
        root_id: root.clone(), task_id: created.task_id.clone(), expected_task_revision: created.task_revision.clone(),
        expected_doc_revision: old_doc, depends_on: Vec::new(),
    }), "task_revision_conflict");
    assert_eq!(fixture.task(&root, &created.task_id).task_revision, created.task_revision);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let independent = fixture.create_task(&root, "Ordinary unfenced append");
    assert!(independent.depends_on.is_empty());
}

#[test]
fn step_actions_execute_on_owned_task_and_prose_edits_preserve_checklist() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (mut task, _, actor) = fixture.working(&root);
    let parent = id();
    let child = id();
    for (step_id, parent_step_id, title) in [
        (parent.clone(), None, "Parent"), (child.clone(), Some(parent.clone()), "Child"),
    ] {
        task = task_result(fixture.apply(&actor, OrchestrationAction::TaskStepAdd {
            root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
            step_id, parent_step_id, before_step_id: None, title: title.into(),
        }).unwrap());
    }
    task = task_result(fixture.apply(&actor, OrchestrationAction::TaskStepRename {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
        step_id: child.clone(), title: "Renamed child".into(),
    }).unwrap());
    task = task_result(fixture.apply(&actor, OrchestrationAction::TaskStepSetChecked {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
        step_id: parent.clone(), checked: true, scope: TaskStepScope::Subtree,
    }).unwrap());
    let protected_steps = task.body[task.body.find("<!-- cockpit-checklist: begin -->").unwrap()..].to_owned();
    task = task_result(fixture.apply(&actor, OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
        title: None, description: Some("New prose retains managed identities and completion".into()),
    }).unwrap());
    assert_eq!(&task.body[task.body.find("<!-- cockpit-checklist: begin -->").unwrap()..], protected_steps);
    task = task_result(fixture.apply(&actor, OrchestrationAction::TaskStepMove {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
        step_id: child.clone(), parent_step_id: None, before_step_id: Some(parent.clone()),
    }).unwrap());
    assert_eq!(task.steps[0].step_id.as_deref(), Some(child.as_str()));
    task = task_result(fixture.apply(&actor, OrchestrationAction::TaskStepRemove {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
        step_id: parent,
    }).unwrap());
    assert_eq!(task.steps.len(), 1);
    assert!(!task.checked);
}

#[test]
fn active_root_and_acceptance_intent_fences_cover_all_content_actions() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let action = OrchestrationAction::TaskCreate {
        root_id: root.clone(), task_id: id(), title: "No preparing-root authoring".into(), description: String::new(),
        depends_on: Vec::new(), follow_up_of: None, expected_doc_revision: None, source_revision: None,
    };
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), action), "invalid_stage");
    let (task, run_id, actor) = fixture.working(&root);
    fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
    fixture.pending_intent(&root, &task, &run_id);
    let root_actor = fixture.launch_and_bind(&root);
    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let bytes = std::fs::read(&path).unwrap();
    for actor in [&root_actor, &Actor::Operator(OperatorOrigin::Browser)] {
        for action in content_actions(&root, &task) {
            assert_code(fixture.apply(actor, action), "intent_conflict");
        }
        assert_code(fixture.apply(actor, OrchestrationAction::TaskDependenciesSet {
            root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
            expected_doc_revision: task_document_revision(&fixture, &root), depends_on: Vec::new(),
        }), "intent_conflict");
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn starting_waiting_work_is_non_destructive_but_checked_or_invalid_tasks_are_rejected_locally() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Waiting prerequisite");
    let waiting = related_task(&fixture, &root, &[source.task_id.clone()], None);
    let run_id = fixture.start(&root, &waiting);
    assert_eq!(fixture.run(&run_id).stage, RunStage::Queued);
    assert!(!fixture.service.dispatch_task_eligible(&fixture.run(&run_id)).unwrap());
    let invalid = fixture.create_task(&root, "Corrupt component");
    let invalid = {
        let locked = fixture.service.store.lock().unwrap();
        let document = locked.tasks(&root).unwrap();
        document.set_dependencies(&invalid.task_id, &invalid.task_revision, &document.doc_revision, &[id()]).unwrap()
    };
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), fixture.start_action(&root, &invalid)),
        "task_dependencies_invalid");
    let independent = fixture.create_task(&root, "Healthy independent component");
    let healthy = fixture.start(&root, &independent);
    fixture.setup(&healthy);
    assert!(fixture.service.dispatch_task_eligible(&fixture.run(&healthy)).unwrap());
    canonical_check(&fixture, &root, &source.task_id, true);
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        fixture.start_action(&root, &fixture.task(&root, &source.task_id))), "task_checked");
}

#[test]
fn staged_private_credential_grants_nothing_until_published_under_the_state_lock() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, mut actor) = fixture.working(&root);
    let (record, path) = credentials::stage_credential(
        fixture.service.base(), &fixture.run(&worker), JobRole::Worker, None,
    ).unwrap();
    caller_mut(&mut actor).auth = credentials::read_job_auth(&path).unwrap();
    let action = report(ReportKind::Progress, None, None);
    let before = fixture.state_bytes();
    assert_code(fixture.apply(&actor, action.clone()), "job_unauthorized");
    assert_eq!(fixture.state_bytes(), before);
    fixture.seed_state(|state| state.credentials.push(record));
    fixture.apply(&actor, action).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
}

#[test]
fn credential_role_root_epoch_and_exclusive_checkout_claim_are_all_fenced() {
    for fence in 0..8 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (_, worker, actor) = fixture.working(&root);
        let Actor::Agent(caller) = &actor else { unreachable!() };
        fixture.seed_state(|state| {
            let index = run_index(state, &worker).unwrap();
            let claim_id = state.runs[index].checkout_claim_id.clone().unwrap();
            let claim_index = state.checkout_claims.iter().position(|c| c.claim_id == claim_id).unwrap();
            let credential = state.credentials.iter_mut()
                .find(|record| record.credential_id == caller.auth.credential_id).unwrap();
            match fence {
                0 => credential.state = credentials::CredentialState::Revoked,
                1 => credential.root_id = id(),
                2 => credential.role = JobRole::Root,
                3 => state.runs[index].attachment.as_mut().unwrap().epoch += 1,
                4 => state.checkout_claims[claim_index].state = claims::ClaimState::Released,
                5 => state.checkout_claims[claim_index].holder.as_mut().unwrap().run_id = id(),
                6 => state.runs[index].checkout.as_mut().unwrap().directory_identity.inode = "replaced-directory".into(),
                7 => {
                    let mut competing = state.checkout_claims[claim_index].clone();
                    competing.claim_id = id();
                    state.checkout_claims.push(competing);
                }
                _ => unreachable!(),
            }
        });
        let before = fixture.state_bytes();
        assert_code(fixture.apply(&actor, report(ReportKind::Progress, None, None)), "job_unauthorized");
        assert_eq!(fixture.state_bytes(), before, "Fence {fence} must not write");
        assert!(fixture.run(&worker).result.is_none());
    }
}

#[test]
fn run_attach_identical_replay_is_no_write_and_changed_payload_conflicts() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let action = OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    };
    let mut receiver = fixture.service.subscribe();
    let first = fixture.apply(&actor, action.clone()).unwrap();
    let OrchestrationActionResult::RunAttached { attach: first } = first.result else {
        panic!("Expected attachment receipt");
    };
    assert!(!first.duplicate);
    let before = fixture.state_bytes();
    let revision = fixture.state().revision;
    assert!(receiver.has_changed().unwrap(), "The first attachment operation must publish its commit");
    assert_eq!(*receiver.borrow_and_update(), revision);
    let replay = fixture.apply(&actor, action.clone()).unwrap();
    let OrchestrationActionResult::RunAttached { attach: replay } = replay.result else {
        panic!("Expected attachment replay");
    };
    assert!(replay.duplicate);
    assert_eq!(replay.run_id, worker);
    assert_eq!(serde_json::to_value(replay.attachment).unwrap(), serde_json::to_value(first.attachment).unwrap());
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().revision, revision);
    assert!(!receiver.has_changed().unwrap());
    for field in 0..3 {
        let mut changed = action.clone();
        let OrchestrationAction::RunAttach { main_session_id, expected_attachment_epoch, attachment_id, .. } = &mut changed else { unreachable!() };
        match field {
            0 => *main_session_id = "changed-conversation".into(),
            1 => *expected_attachment_epoch += 1,
            2 => *attachment_id = id(),
            _ => unreachable!(),
        }
        assert_code(fixture.apply(&actor, changed), "operation_conflict");
        assert_eq!(fixture.state_bytes(), before);
    }
}

#[test]
fn run_resume_identical_replay_preserves_protected_writer_and_changed_payload_conflicts() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let other_root = fixture.root();
    let original = fixture.run(&worker);
    let old_claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| original.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    let action = OrchestrationAction::RunResume {
        run_id: worker.clone(), operation_id: id(),
        expected_attachment_epoch: Some(original.attachment.as_ref().unwrap().epoch),
        mode: ResumeMode::ReattachOrRestart, decision_id: None,
    };
    let mut receiver = fixture.service.subscribe();
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::RunResumed { resume: first } = first.result else {
        panic!("Expected resume receipt");
    };
    assert!(!first.duplicate);
    assert_eq!(first.resume.phase, ResumePhase::Blocked);
    assert_eq!(first.resume.cause.as_ref().unwrap().code, "writer_exit_unproven");
    assert_eq!(first.resume.cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
    assert!(first.resume.decision_id.is_none(), "Missing observation is not a living-writer replacement decision");
    let state = fixture.state();
    let operation = state.operations.iter().find(|record| record.operation_id == first.resume.operation_id).unwrap();
    assert_eq!(operation.run_id.as_deref(), Some(worker.as_str()));
    assert_eq!(operation.resume_operation_id.as_deref(), Some(first.resume.operation_id.as_str()));
    assert!(operation.result.is_none());
    assert!(operation.effect_receipts.is_empty());
    let blocked = fixture.run(&worker);
    assert_eq!(serde_json::to_value(blocked.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
    assert_eq!(blocked.checkout_claim_id, original.checkout_claim_id);
    let current_claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| claim.claim_id == old_claim.claim_id).unwrap();
    assert_eq!(serde_json::to_value(current_claim).unwrap(), serde_json::to_value(old_claim).unwrap());
    let before = fixture.state_bytes();
    let revision = fixture.state().revision;
    assert!(receiver.has_changed().unwrap(), "The first resume operation must publish its commit");
    assert_eq!(*receiver.borrow_and_update(), revision);
    let replay = fixture.operator(action.clone());
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else {
        panic!("Expected resume replay");
    };
    assert!(replay.duplicate);
    assert_eq!(serde_json::to_value(replay.resume).unwrap(), serde_json::to_value(first.resume).unwrap());
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().revision, revision);
    assert!(!receiver.has_changed().unwrap());
    for field in 0..4 {
        let mut changed = action.clone();
        let OrchestrationAction::RunResume { run_id, expected_attachment_epoch, mode, decision_id, .. } = &mut changed else { unreachable!() };
        match field {
            0 => *expected_attachment_epoch = Some(original.attachment.as_ref().unwrap().epoch + 1),
            1 => *mode = ResumeMode::ReplaceExisting,
            2 => *decision_id = Some(id()),
            3 => *run_id = other_root.clone(),
            _ => unreachable!(),
        }
        assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
        assert_eq!(fixture.state_bytes(), before);
    }
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    assert_eq!(fixture.run(&worker).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
}

#[test]
fn supervisor_start_another_uses_immutable_project_and_replays_its_operation() {
    let fixture = Fixture::new();
    let action = OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::Another, target: Some(target()),
        label: Some("Explicit another coordinator".into()), selected_root_id: None,
        expected_project_key: Some(fixture.project().key),
    };
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::SupervisorStarted { start: first } = first.result else {
        panic!("Expected supervisor start");
    };
    assert!(!first.duplicate);
    assert_eq!(fixture.run(&first.root_id).project, Some(fixture.project()));
    assert_eq!(fixture.run(&first.root_id).attachment.as_ref().unwrap().epoch, 1);
    assert!(fixture.run(&first.root_id).checkout_claim_id.is_none());
    let before = fixture.state_bytes();
    let replay = fixture.operator(action.clone());
    let OrchestrationActionResult::SupervisorStarted { start: replay } = replay.result else {
        panic!("Expected supervisor replay");
    };
    assert!(replay.duplicate);
    assert_eq!(replay.root_id, first.root_id);
    assert_eq!(fixture.state_bytes(), before);
    let mut changed = action;
    let OrchestrationAction::SupervisorStart { label, .. } = &mut changed else { unreachable!() };
    *label = Some("Changed request under the same operation ID".into());
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn concurrent_reuse_starts_match_or_create_one_open_project_root() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let barrier = std::sync::Barrier::new(3);
    let revision = fixture.state().revision;
    let roots = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let configuration = &fixture.configuration;
            let project = project.clone();
            let barrier = &barrier;
            let operation_id = id();
            workers.push(scope.spawn(move || {
                let service = OrchestrationService::open(configuration).unwrap();
                barrier.wait();
                let response = service.mutate_resolved(
                    &Actor::Operator(OperatorOrigin::Browser),
                    OrchestrationMutationRequest {
                        session_id: SESSION.into(), expected_revision: Some(revision),
                        action: OrchestrationAction::SupervisorStart {
                            operation_id, mode: SupervisorStartMode::ReuseOrResume, target: Some(target()),
                            label: Some("Concurrent project coordinator".into()), selected_root_id: None,
                            expected_project_key: Some(project.key.clone()),
                        },
                    },
                    &ResolvedMutationInput { project: Some(project), ..ResolvedMutationInput::default() },
                ).unwrap();
                let OrchestrationActionResult::SupervisorStarted { start } = response.result else {
                    panic!("Expected project start");
                };
                start.root_id
            }));
        }
        barrier.wait();
        workers.into_iter().map(|worker| worker.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(roots[0], roots[1]);
    assert_eq!(fixture.state().runs.iter().filter(|run| run.parent_run_id.is_none()
        && run.stage != RunStage::Closed && run.project.as_ref() == Some(&project)).count(), 1);
    assert_eq!(fixture.state().operations.iter()
        .filter(|record| record.kind == store::DurableOperationKind::SupervisorStart).count(), 2);
}

#[test]
fn child_registration_requires_physical_identity_and_replays_without_reminting_credentials() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, parent) = fixture.working(&root);
    let mut registering = parent.clone();
    let caller = caller_mut(&mut registering);
    caller.agent_kind = Some(AgentKind::Subagent);
    caller.subagent_id = Some("native-child".into());
    caller.omp_session_id = Some("native-child-conversation".into());
    let action = OrchestrationAction::JobChildRegister {
        operation_id: id(), child_id: "native-child".into(),
        child_session_id: "native-child-conversation".into(), parent_child_id: None,
    };
    for fence in 0..4 {
        let mut forged = registering.clone();
        let caller = caller_mut(&mut forged);
        match fence {
            0 => caller.process.as_mut().unwrap().start_ticks += 1,
            1 => caller.actual_agent_kind = Some(NativeAgentKind::Other("shell".into())),
            2 => caller.main_omp_session_id = Some("not-bound-main".into()),
            3 => caller.subagent_id = Some("different-child".into()),
            _ => unreachable!(),
        }
        let before = fixture.state_bytes();
        assert_code(fixture.apply(&forged, action.clone()), "child_identity_unproven");
        assert_eq!(fixture.state_bytes(), before);
    }
    let mut receiver = fixture.service.subscribe();
    fixture.apply(&registering, action.clone()).unwrap();
    let credentials = fixture.state().credentials.into_iter().filter(|record|
        record.run_id == worker && record.role == JobRole::Child
            && record.subject_child_id.as_deref() == Some("native-child")).collect::<Vec<_>>();
    assert_eq!(credentials.len(), 1);
    let Actor::Agent(main) = &parent else { unreachable!() };
    let path = fixture.service.job_child_credential_file(main, "native-child").unwrap();
    let child_auth = credentials::read_job_auth(&path).unwrap();
    let credential_bytes = std::fs::read(&path).unwrap();
    assert_eq!(child_auth.credential_id, credentials[0].credential_id);
    let before = fixture.state_bytes();
    let revision = fixture.state().revision;
    assert!(receiver.has_changed().unwrap(), "The first registration must publish its commit");
    assert_eq!(*receiver.borrow_and_update(), revision);
    fixture.apply(&registering, action.clone()).unwrap();
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().revision, revision);
    assert_eq!(std::fs::read(&path).unwrap(), credential_bytes, "Replay cannot remint the child token");
    assert!(!receiver.has_changed().unwrap());
    for field in 0..3 {
        let mut changed = action.clone();
        let OrchestrationAction::JobChildRegister { child_id, child_session_id, parent_child_id, .. } = &mut changed else { unreachable!() };
        match field {
            0 => *child_id = "changed-child".into(),
            1 => *child_session_id = "changed-child-session".into(),
            2 => *parent_child_id = Some("changed-parent".into()),
            _ => unreachable!(),
        }
        assert_code(fixture.apply(&registering, changed), "operation_conflict");
        assert_eq!(fixture.state_bytes(), before);
    }
    let caller = caller_mut(&mut registering);
    caller.auth = child_auth;
    assert_eq!(fixture.service.job_child_credential_file(caller, "native-child").unwrap_err().code,
        "actor_forbidden");
    assert_code(fixture.apply(&registering, action), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn run_attach_never_replaces_physical_process_with_a_bearer_credential() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let Actor::Agent(main) = &actor else { unreachable!() };
    let original = fixture.run(&worker);
    let action = OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: main.auth.attachment_id.clone(),
        expected_attachment_epoch: main.auth.attachment_epoch,
        main_session_id: main.omp_session_id.clone().unwrap(),
    };
    for fence in 0..4 {
        let mut forged = actor.clone();
        let caller = caller_mut(&mut forged);
        match fence {
            0 => caller.process.as_mut().unwrap().start_ticks += 1,
            1 => caller.process = None,
            2 => caller.actual_agent_kind = Some(NativeAgentKind::Other("shell".into())),
            3 => caller.omp_session_id = Some("other-conversation".into()),
            _ => unreachable!(),
        }
        let before = fixture.state_bytes();
        assert_code(fixture.apply(&forged, action.clone()), "attachment_conflict");
        assert_eq!(fixture.state_bytes(), before);
        assert_eq!(fixture.run(&worker).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
        assert_eq!(fixture.run(&worker).checkout_claim_id, original.checkout_claim_id);
    }
}

#[test]
fn reuse_start_prefers_selected_matching_root_then_stable_latest_open_root() {
    let fixture = Fixture::new();
    let first = fixture.root();
    let second = fixture.root();
    for root in [&first, &second] {
        fixture.seed_run(root, |run| run.updated_at = "2026-10-10T00:00:00Z".into());
    }
    let state = fixture.state();
    let latest = preferred_project_root(&state.runs, SESSION, &fixture.project().key, None).unwrap();
    let preferred = state.runs[latest].run_id.clone();
    // Equal timestamps have a deterministic UUID lexical preference.
    assert_eq!(preferred.as_str(), first.as_str().min(second.as_str()));
    let other = if preferred == first { second.clone() } else { first.clone() };
    let selected = fixture.operator(OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
        selected_root_id: Some(other.clone()), expected_project_key: Some(fixture.project().key),
    });
    let OrchestrationActionResult::SupervisorStarted { start } = selected.result else { unreachable!() };
    assert_eq!(start.root_id, other);
    assert_eq!(fixture.state().runs.len(), 2);
    fixture.seed_run(&preferred, |run| run.updated_at = "2026-10-11T00:00:00Z".into());
    let latest = fixture.operator(OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
        selected_root_id: Some(id()), expected_project_key: Some(fixture.project().key),
    });
    let OrchestrationActionResult::SupervisorStarted { start } = latest.result else { unreachable!() };
    assert_eq!(start.root_id, preferred);
    fixture.operator(OrchestrationAction::CancelRun { run_id: preferred.clone() });
    let open = fixture.operator(OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
        selected_root_id: Some(preferred), expected_project_key: Some(fixture.project().key),
    });
    let OrchestrationActionResult::SupervisorStarted { start } = open.result else { unreachable!() };
    assert_eq!(start.root_id, other, "Closed history is not implicitly reopened");
    assert_eq!(fixture.state().runs.len(), 2);
}

#[test]
fn obsolete_schema_reads_fresh_current_state_and_replaces_records_only_on_ordinary_save() {
    let fixture = Fixture::new();
    let source = serde_json::to_vec(&serde_json::json!({
        "schema": 1, "revision": 17, "runs": [{"obsolete": "permission-era state"}],
        "messages": [], "subagents": [], "task_intents": [], "assignment_intents": [],
    })).unwrap();
    {
        let locked = fixture.service.store.lock().unwrap();
        locked.state_dir().write("state.json", &source).unwrap();
    }
    let service = OrchestrationService::open(&fixture.configuration).unwrap();
    let state = service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.schema, store::SCHEMA);
    assert_eq!(state.revision, 17);
    assert!(state.runs.is_empty());
    assert!(state.operations.is_empty());
    assert!(state.credentials.is_empty() && state.checkout_claims.is_empty());
    assert_eq!(fixture.state_bytes(), source, "Read-only open does not rewrite obsolete state");
    let response = service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: Some(17),
            action: OrchestrationAction::SupervisorStart {
                operation_id: id(), mode: SupervisorStartMode::Another, target: Some(target()),
                label: None, selected_root_id: None, expected_project_key: Some(fixture.project().key),
            },
        }, &ResolvedMutationInput { project: Some(fixture.project()), ..ResolvedMutationInput::default() }).unwrap();
    assert_eq!(response.revision, 18);
    let state = service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.schema, store::SCHEMA);
    assert_eq!(state.runs.len(), 1);
    assert_eq!(state.operations.len(), 1);
    assert_ne!(fixture.state_bytes(), source);
}

#[test]
fn runtime_composition_does_not_hide_malformed_or_unsupported_store_errors() {
    for source in [b"not-json".as_slice(), br#"{"schema":99,"revision":17}"#.as_slice()] {
        let fixture = Fixture::new();
        {
            let locked = fixture.service.store.lock().unwrap();
            locked.state_dir().write("state.json", source).unwrap();
        }
        match OrchestrationService::open(&fixture.configuration) {
            Err(error) => assert_eq!(error.code, "state_corrupt"),
            Ok(_) => panic!("Malformed or unsupported state must not open"),
        }
        assert_eq!(fixture.state_bytes(), source);
    }
}

#[test]
fn physical_resume_preserves_current_writer_but_needs_an_authenticated_extension() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, mut actor) = fixture.working(&root);
    let original = fixture.run(&worker);
    let observed = original.attachment.clone().unwrap();
    let claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| original.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    fixture.seed_state(|state| {
        state.credentials.retain(|record| record.run_id != worker);
        state.operations.retain(|record| record.kind != store::DurableOperationKind::RunAttach
            || record.actor_run_id.as_deref() != Some(worker.as_str()));
    });
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
    for staged in [false, true] {
        if staged {
            caller_mut(&mut actor).auth = fixture.auth(&worker, JobRole::Worker, None);
        }
        let response = fixture.resume_observed(&worker, observed.clone());
        assert_eq!(response.resume.phase, ResumePhase::Blocked);
        assert_eq!(response.resume.cause.as_ref().unwrap().code, "current_extension_unavailable");
        assert_eq!(response.resume.cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
        assert!(response.resume.decision_id.is_none());
        let current = fixture.run(&worker);
        let attachment = current.attachment.as_ref().unwrap();
        assert_eq!(attachment.state, AttachmentState::Disconnected);
        assert_eq!(attachment.attachment_id, observed.attachment_id);
        assert_eq!(attachment.epoch, observed.epoch);
        assert_eq!(attachment.process, observed.process);
        assert_eq!(attachment.shell, observed.shell);
        assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
        assert_eq!(serde_json::to_value(&current.dispatch).unwrap(), serde_json::to_value(&original.dispatch).unwrap());
        let protected = fixture.state().checkout_claims.into_iter()
            .find(|record| record.claim_id == claim.claim_id).unwrap();
        assert_eq!(serde_json::to_value(protected).unwrap(), serde_json::to_value(&claim).unwrap());
        assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker),
            "Staged credential alone is not evidence that this living OMP loaded the extension");
    }
    // Ordinary durable authority stays available while semantic connection is absent.
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    assert_eq!(fixture.run(&worker).attachment.as_ref().unwrap().state, AttachmentState::Disconnected);
    let blocked = fixture.run(&worker).resume.unwrap();
    let Actor::Agent(caller) = &actor else { unreachable!() };
    fixture.apply(&actor, OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    }).unwrap();
    let connected = fixture.run(&worker);
    assert_eq!(connected.attachment.as_ref().unwrap().state, AttachmentState::Connected);
    assert_eq!(connected.attachment.as_ref().unwrap().epoch, observed.epoch);
    assert_eq!(connected.attachment.as_ref().unwrap().process, observed.process);
    assert_eq!(connected.checkout_claim_id, original.checkout_claim_id);
    assert_eq!(serde_json::to_value(&connected.dispatch).unwrap(), serde_json::to_value(&original.dispatch).unwrap());
    let resumed = connected.resume.as_ref().unwrap();
    assert_eq!(resumed.operation_id, blocked.operation_id);
    assert_eq!(resumed.phase, ResumePhase::Reattached);
    assert!(resumed.cause.is_none(), "Authenticated registration clears unavailable UX immediately");
    assert!(resumed.replacement_attachment_epoch.is_none());
    let state = fixture.state();
    let operation = state.operations.iter().find(|record| record.operation_id == blocked.operation_id).unwrap();
    assert_eq!(operation.run_id.as_deref(), Some(worker.as_str()));
    assert_eq!(operation.resume_operation_id.as_deref(), Some(blocked.operation_id.as_str()));
    assert!(operation.result.is_none());
    assert!(operation.effect_receipts.is_empty());
    let protected = state.checkout_claims.iter().find(|record| record.claim_id == claim.claim_id).unwrap();
    assert_eq!(serde_json::to_value(protected).unwrap(), serde_json::to_value(&claim).unwrap());
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
    let before = fixture.state_bytes();
    let replay = fixture.operator(OrchestrationAction::RunResume {
        run_id: worker.clone(), operation_id: blocked.operation_id,
        expected_attachment_epoch: Some(observed.epoch),
        mode: ResumeMode::ReattachOrRestart, decision_id: None,
    });
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.resume.phase, ResumePhase::Reattached);
    assert!(replay.resume.cause.is_none());
    assert_eq!(fixture.state_bytes(), before, "Owner replay returns the immediately recovered receipt without relaunch");
}

#[test]
fn physical_start_observation_without_current_extension_binding_does_not_reuse_a_root() {
    for staged in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        fixture.launch_and_bind(&root);
        let original = fixture.run(&root);
        let observed = original.attachment.clone().unwrap();
        fixture.seed_state(|state| {
            state.credentials.retain(|record| record.run_id != root);
            state.operations.retain(|record| record.kind != store::DurableOperationKind::RunAttach
                || record.actor_run_id.as_deref() != Some(root.as_str()));
        });
        if staged {
            fixture.auth(&root, JobRole::Root, None);
        }
        let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            OrchestrationMutationRequest {
                session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
                action: OrchestrationAction::SupervisorStart {
                    operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
                    selected_root_id: Some(root.clone()), expected_project_key: Some(fixture.project().key),
                },
            }, &ResolvedMutationInput {
                project: Some(fixture.project()), attachment_observation: Some(observed.clone()),
                ..ResolvedMutationInput::default()
            }).unwrap();
        let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
        assert_eq!(start.root_id, root);
        assert_eq!(start.outcome, SupervisorStartOutcome::Resuming);
        let current = fixture.run(&root);
        assert_eq!(current.resume.as_ref().unwrap().phase, ResumePhase::Blocked);
        assert_eq!(current.resume.as_ref().unwrap().cause.as_ref().unwrap().code, "current_extension_unavailable");
        assert_eq!(current.resume.as_ref().unwrap().cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
        let attachment = current.attachment.as_ref().unwrap();
        assert_eq!(attachment.state, AttachmentState::Disconnected);
        assert_eq!(attachment.epoch, observed.epoch);
        assert_eq!(attachment.process, observed.process);
        assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
        assert_eq!(fixture.state().runs.len(), 1);
        assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
    }
}

#[test]
fn authenticated_current_epoch_attach_allows_physical_reattach_without_launch_or_claim_transfer() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let original = fixture.run(&worker);
    let observed = original.attachment.clone().unwrap();
    let claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| original.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    let (_, _, bindings) = fixture.service.recorded_runs_with_bindings(SESSION).unwrap();
    assert_eq!(bindings.get(&worker), Some(&(observed.attachment_id.clone(), observed.epoch)));
    let result = fixture.resume_observed(&worker, observed.clone());
    assert_eq!(result.resume.phase, ResumePhase::Reattached);
    assert!(result.resume.replacement_attachment_epoch.is_none());
    assert!(result.resume.cause.is_none());
    let current = fixture.run(&worker);
    assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
    assert_eq!(serde_json::to_value(current.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
    assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
    let retained = fixture.state().checkout_claims.into_iter().find(|record| record.claim_id == claim.claim_id).unwrap();
    assert_eq!(serde_json::to_value(retained).unwrap(), serde_json::to_value(claim).unwrap());
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
}

#[test]
fn revoked_credentials_or_stale_attach_receipts_never_prove_current_extension_binding() {
    for fence in 0..2 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (_, worker, _) = fixture.working(&root);
        let original = fixture.run(&worker);
        let observed = original.attachment.clone().unwrap();
        fixture.seed_state(|state| {
            if fence == 0 {
                for credential in state.credentials.iter_mut().filter(|record| record.run_id == worker) {
                    credential.state = credentials::CredentialState::Revoked;
                }
            } else {
                for operation in state.operations.iter_mut().filter(|record|
                    record.kind == store::DurableOperationKind::RunAttach
                        && record.actor_run_id.as_deref() == Some(worker.as_str())) {
                    if let Some(OrchestrationActionResult::RunAttached { attach }) = &mut operation.result {
                        attach.attachment.epoch += 1;
                    }
                }
            }
        });
        assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
        let result = fixture.resume_observed(&worker, observed.clone());
        assert_eq!(result.resume.phase, ResumePhase::Blocked);
        assert_eq!(result.resume.cause.as_ref().unwrap().code, "current_extension_unavailable");
        assert_eq!(result.resume.cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
        let current = fixture.run(&worker);
        assert_eq!(current.attachment.as_ref().unwrap().epoch, observed.epoch);
        assert_eq!(current.attachment.as_ref().unwrap().process, observed.process);
        assert_eq!(current.attachment.as_ref().unwrap().state, AttachmentState::Disconnected);
        assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
        assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
    }
}

#[test]
fn physical_start_reuses_only_a_root_with_applied_current_epoch_attachment_binding() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.launch_and_bind(&root);
    let original = fixture.run(&root);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    fixture.apply(&actor, OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    }).unwrap();
    let observed = fixture.run(&root).attachment.unwrap();
    let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
            action: OrchestrationAction::SupervisorStart {
                operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
                selected_root_id: Some(root.clone()), expected_project_key: Some(fixture.project().key),
            },
        }, &ResolvedMutationInput {
            project: Some(fixture.project()), attachment_observation: Some(observed.clone()),
            ..ResolvedMutationInput::default()
        }).unwrap();
    let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
    assert_eq!(start.root_id, root);
    assert_eq!(start.outcome, SupervisorStartOutcome::Reused);
    assert!(start.resume_operation_id.is_none());
    let current = fixture.run(&root);
    assert_eq!(current.attachment.as_ref().unwrap().epoch, observed.epoch);
    assert_eq!(current.attachment.as_ref().unwrap().process, observed.process);
    assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
    assert_eq!(fixture.state().runs.len(), 1);
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
}

#[test]
fn closed_history_with_a_recorded_physical_process_cannot_be_adopted_as_another_root() {
    let fixture = Fixture::new();
    let seed = fixture.root();
    let actor = fixture.launch_tab(&seed);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let bootstrap = BootstrapCaller {
        endpoint_identity: caller.endpoint_identity.clone(), session_id: caller.session_id.clone(),
        workspace_id: caller.workspace_id.clone(), tab_id: "fresh-adoption-tab".into(),
        pane_id: "fresh-adoption-pane".into(), boot_id: caller.boot_id.clone(),
        terminal_id: Some("fresh-adoption-terminal".into()),
        native_session_id: Some("fresh-adoption-native".into()), omp_session_id: Some("fresh-adoption-native".into()),
        agent_kind: Some(AgentKind::Main), actual_agent_kind: caller.actual_agent_kind.clone(),
        process: Some(live_process()),
    };
    let adopted = run_result(fixture.apply(&Actor::Bootstrap(bootstrap.clone()),
        OrchestrationAction::RunAdopt { label: "Fresh unbound process".into() }).unwrap()).0;
    assert_eq!(fixture.run(&adopted).kind, RunKind::Adopted);
    assert_eq!(fixture.run(&adopted).attachment.as_ref().unwrap().process, bootstrap.process);
    fixture.seed_run(&adopted, |run| {
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Cancelled);
    });
    let before = fixture.state_bytes();
    let state = fixture.state();
    assert_eq!(fixture.run(&adopted).attachment.as_ref().unwrap().process, bootstrap.process,
        "Closing tracking is not proof that its physical process exited");
    assert_code(fixture.apply(&Actor::Bootstrap(bootstrap),
        OrchestrationAction::RunAdopt { label: "Must not duplicate the retained incarnation".into() }), "caller_mismatch");
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().runs.len(), state.runs.len());
    assert_eq!(fixture.state().credentials.len(), state.credentials.len());
}

#[test]
fn resume_join_replays_current_owner_and_settles_old_receipt_before_later_resume() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, _) = fixture.working(&root);
    let response = fixture.resume_exited(&Actor::Operator(OperatorOrigin::Browser), &worker);
    let OrchestrationActionResult::RunResumed { resume: first } = response.result else { unreachable!() };
    assert_eq!(first.resume.phase, ResumePhase::Launching);
    let joined_action = fixture.resume_action(&worker);
    let joined = fixture.operator(joined_action.clone());
    let OrchestrationActionResult::RunResumed { resume: joined } = joined.result else { unreachable!() };
    assert_eq!(joined.resume.operation_id, first.resume.operation_id);
    let OrchestrationAction::RunResume { operation_id: joined_id, .. } = &joined_action else { unreachable!() };
    let state = fixture.state();
    let owner = state.operations.iter().find(|record| record.operation_id == first.resume.operation_id).unwrap();
    assert!(owner.result.is_none());
    assert_eq!(owner.effect_receipts.len(), 2);
    let join = state.operations.iter().find(|record| &record.operation_id == joined_id).unwrap();
    assert_eq!(join.run_id.as_deref(), Some(worker.as_str()));
    assert_eq!(join.resume_operation_id.as_deref(), Some(first.resume.operation_id.as_str()));
    assert!(join.result.is_none());
    assert!(join.effect_receipts.is_empty());
    fixture.launch_and_bind(&worker);
    assert_eq!(fixture.run(&worker).resume.as_ref().unwrap().phase, ResumePhase::Connected);
    let bytes = fixture.state_bytes();
    let replay = fixture.operator(joined_action.clone());
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.resume.phase, ResumePhase::Connected);
    assert_eq!(fixture.state_bytes(), bytes);
    let later = fixture.operator(fixture.resume_action(&worker));
    let OrchestrationActionResult::RunResumed { resume: later } = later.result else { unreachable!() };
    assert_ne!(later.resume.operation_id, first.resume.operation_id);
    assert_eq!(later.resume.phase, ResumePhase::Blocked);
    let state = fixture.state();
    for operation_id in [&first.resume.operation_id, joined_id] {
        let operation = state.operations.iter().find(|record| &record.operation_id == operation_id).unwrap();
        let Some(OrchestrationActionResult::RunResumed { resume }) = &operation.result else { unreachable!() };
        assert_eq!(resume.resume.operation_id, first.resume.operation_id);
        assert_eq!(resume.resume.phase, ResumePhase::Connected);
    }
    let bytes = fixture.state_bytes();
    let replay = fixture.operator(joined_action);
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.resume.operation_id, first.resume.operation_id);
    assert_eq!(replay.resume.phase, ResumePhase::Connected);
    assert_eq!(fixture.state_bytes(), bytes);
    let state = fixture.state();
    let owner = state.operations.iter().find(|record| record.operation_id == first.resume.operation_id).unwrap();
    assert_eq!(owner.effect_receipts.len(), 2, "Replays never rearchive launch/setup effects");
}

#[test]
fn worker_can_resume_own_job_but_child_cannot_and_replace_never_stops_a_same_boot_writer() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let child = fixture.child_actor(&actor, &worker, "resume-child");
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&child, fixture.resume_action(&worker)), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), bytes);
    let action = fixture.resume_action(&worker);
    let response = fixture.apply(&actor, action).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
    assert_eq!(resume.resume.phase, ResumePhase::Blocked);
    // A decision is specific to this observed protected incarnation.
    let response = fixture.service.advance_resume_observed(&worker, &resume.resume.operation_id,
        &ResolvedMutationInput {
            observation_cause: Some(ResumeCause {
                code: "living_writer".into(), message: "Current writer is alive".into(),
                class: ResumeCauseClass::Decision,
            }), ..ResolvedMutationInput::default()
        }).unwrap();
    let original = fixture.run(&worker);
    let decision = response.resume.decision_id.unwrap();
    let response = fixture.apply(&actor, OrchestrationAction::RunResume {
        run_id: worker.clone(), operation_id: id(),
        expected_attachment_epoch: original.attachment.as_ref().map(|attachment| attachment.epoch),
        mode: ResumeMode::ReplaceExisting, decision_id: Some(decision),
    }).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
    assert_eq!(resume.resume.phase, ResumePhase::Blocked);
    assert_eq!(resume.resume.cause.as_ref().unwrap().code, "same_boot_replacement_unsupported");
    let current = fixture.run(&worker);
    assert_review_fields_only(&original, &current);
    assert!(current.retirement.is_none());
    assert_ne!(current.attachment.as_ref().unwrap().state, AttachmentState::Quiescing);
    let state = fixture.state();
    let operation = state.operations.iter().find(|record| record.operation_id == resume.resume.operation_id).unwrap();
    assert!(operation.result.is_none());
    assert!(operation.effect_receipts.is_empty());
    assert!(state.checkout_claims.iter().all(|claim| claim.state != claims::ClaimState::Released));
    assert!(state.credentials.iter().filter(|credential| credential.run_id == worker)
        .all(|credential| credential.state == credentials::CredentialState::Active));
}

#[test]
fn private_attachment_after_native_launch_proof_starts_once_and_sends_one_brief() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Late private attachment");
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let actor = fixture.launch_tab(&worker);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    // Native runtime proof is physical evidence, not an applied extension attach.
    fixture.seed_run(&worker, |run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.main_session_id = caller.omp_session_id.clone();
        attachment.process = caller.process.clone();
    });
    fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Starting);
    assert!(fixture.state().messages.iter().all(|message|
        message.to_run_id != worker || message.kind != MessageKind::WorkBrief));
    let attach = OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    };
    fixture.apply(&actor, attach.clone()).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
    let bytes = fixture.state_bytes();
    fixture.apply(&actor, attach).unwrap();
    assert_eq!(fixture.state_bytes(), bytes);
    let briefs = fixture.state().messages.into_iter().filter(|message|
        message.to_run_id == worker && message.kind == MessageKind::WorkBrief).collect::<Vec<_>>();
    assert_eq!(briefs.len(), 1);
    assert_eq!(briefs[0].text, fixture.run(&worker).brief);
}

#[test]
fn worker_start_requires_current_canonical_task_and_document_revisions_without_writes() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Start revision fence");
    let mut action = fixture.start_action(&root, &task);
    fixture.create_task(&root, "Unrelated document edit");
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), action.clone()),
        "task_revision_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    let OrchestrationAction::WorkerStart { expected_doc_revision, .. } = &mut action else { unreachable!() };
    *expected_doc_revision = task_document_revision(&fixture, &root);
    fixture.operator(OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision,
        title: None, description: Some("Changed canonical scope before start".into()),
    });
    let OrchestrationAction::WorkerStart { expected_doc_revision, .. } = &mut action else { unreachable!() };
    *expected_doc_revision = task_document_revision(&fixture, &root);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), action),
        "task_revision_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    assert!(fixture.state().runs.iter().all(|run| run.kind != RunKind::Worker));
}
