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
            orchestration: cockpit_protocol::projects::OrchestrationConfiguration {
                harness_kind: "service-test".into(),
                command: "test-agent".into(),
                extra_args: vec!["fresh".into()],
                resume_args: Some(vec!["resume".into(), "{session_id}".into()]),
                inbox_prompt_hints: false,
                routes: Vec::new(),
                ..Default::default()
            },
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

    fn auth(&self, run_id: &str, role: JobRole) -> JobAuth {
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
            credentials::stage_credential(self.service.base(), &state.runs[index], role).unwrap();
        state.credentials.push(credential);
        locked.save(&mut state).unwrap();
        credentials::read_job_auth(&path).unwrap()
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
        self.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser), OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: Some(self.state().revision),
            action: OrchestrationAction::RunAttach {
                operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
                expected_attachment_epoch: caller.auth.attachment_epoch,
                conversation: caller.conversation.clone().unwrap(),
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
        }
    }

    fn resume_exited(&self, actor: &Actor, run_id: &str) -> OrchestrationMutationResponse {
        let run = self.run(run_id);
        let attachment = run.attachment.as_ref().unwrap();
        assert_eq!(run.kind, RunKind::Worker, "Root recovery tests use the dedicated physical observer");
        let claim = self.state().checkout_claims.into_iter()
            .find(|claim| run.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
        let (claim_revision, checkout) = (claim.revision, Some(claim.checkout));
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
            basis: claims::ExitBasis::Reboot,
        };
        self.service.mutate_resolved(actor, OrchestrationMutationRequest {
            session_id: SESSION.into(),
            expected_revision: matches!(actor, Actor::Operator(_)).then(|| self.state().revision),
            action: OrchestrationAction::RunResume {
                run_id: run_id.into(), operation_id: id(), expected_attachment_epoch: Some(attachment.epoch),
            },
        }, &ResolvedMutationInput { writer_exit_proof: Some(proof), ..ResolvedMutationInput::default() }).unwrap()
    }
    fn create_task(&self, root_id: &str, title: &str) -> Task {
        if self.run(root_id).stage == RunStage::Starting {
            self.root_actor(root_id);
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
        let actor = self.root_actor(&root);
        let run_id = run_result(self.apply(&actor, self.start_action(&root, task)).unwrap()).0;
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
    fn root_actor(&self, root_id: &str) -> Actor {
        let actor = self.neutral_actor(root_id);
        self.apply(&actor, OrchestrationAction::RunRegister {
            operation_id: caller_auth(&actor).credential_id.clone(),
        }).unwrap();
        actor
    }

    fn neutral_actor(&self, run_id: &str) -> Actor {
        let run = self.run(run_id);
        let auth = self.auth(run_id, if run.kind == RunKind::Worker { JobRole::Worker } else { JobRole::Root });
        Actor::Agent(AgentCaller {
            assignment_generation: auth.assignment_generation,
            auth,
            conversation: Some(run.attachment.as_ref().and_then(|attachment| attachment.conversation.clone())
                .unwrap_or_else(|| ConversationIdentity { harness: run.launch_config.harness_kind.clone(), session_id: format!("opaque-chat-{run_id}") })),
            endpoint_identity: String::new(), session_id: SESSION.into(),
            workspace_id: String::new(), tab_id: String::new(), pane_id: String::new(),
            boot_id: None, terminal_id: None, native_session_id: None,
            actual_agent_kind: None, process: None,
        })
    }

    fn launch_and_bind(&self, run_id: &str) -> Actor {
        let actor = self.launch_tab(run_id);
        self.service.record_launch_pending(&self.run(run_id)).unwrap();
        self.apply(&actor, OrchestrationAction::RunRegister {
            operation_id: caller_auth(&actor).credential_id.clone(),
        }).unwrap();
        self.service.record_launch_verified(&self.run(run_id)).unwrap();
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
            peer_process: None,
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
                    staging_workspace_id: None,
                    scope_unit: None,
                    latch: None,
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
                    process_scope: None,
                },
            )
            .unwrap();
        self.seed_run(run_id, |run| {
            let attachment = run.attachment.as_mut().unwrap();
            attachment.machine_id = Some("test-machine".into());
            attachment.process = Some(NativeProcessIdentity {
                pid: 42, start_ticks: 123, kernel_boot_id: Some("kernel-one".into()),
            });
        });
        Actor::Agent(AgentCaller {
            auth: self.auth(run_id, if run.kind == RunKind::Worker { JobRole::Worker } else { JobRole::Root }),
            assignment_generation: run.assignment_generation,
            conversation: Some(run.attachment.as_ref().and_then(|attachment| attachment.conversation.clone())
                .unwrap_or_else(|| ConversationIdentity { harness: run.launch_config.harness_kind.clone(), session_id: format!("opaque-chat-{run_id}") })),
            endpoint_identity: location.endpoint_identity,
            session_id: SESSION.into(),
            workspace_id: location.workspace_id,
            tab_id: location.tab_id,
            pane_id: location.pane_id,
            boot_id: location.boot_id,
            terminal_id: location.terminal_id,
            native_session_id: location.native_session_id,
            actual_agent_kind: Some(NativeAgentKind::Other(run.launch_config.harness_kind.clone())),
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
            conversation: None,
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

fn caller_auth(actor: &Actor) -> &JobAuth {
    match actor {
        Actor::Agent(caller) => &caller.auth,
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

mod attachment;
mod launch;
mod lifecycle;
mod result_recovery;
mod task_ownership;
mod wait_recovery;

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

fn assert_agent_decision(message: &Message, run_id: &str) {
    assert!(matches!(&message.from, ActorRef::Run { run_id: from } if from == run_id));
    assert!(!message.stale);
}

fn task_document_revision(fixture: &Fixture, root: &str) -> String {
    fixture.service.store.lock().unwrap().tasks(root).unwrap().doc_revision.clone()
}
