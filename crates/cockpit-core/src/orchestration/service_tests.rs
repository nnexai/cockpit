use super::*;
use std::path::PathBuf;

const SESSION: &str = "isolated-service-test";

struct Fixture {
    root: PathBuf,
    configuration: ProjectConfiguration,
    service: OrchestrationService,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("cockpit-orchestration-service-{}", Uuid::new_v4()));
        let configuration = ProjectConfiguration {
            repository_roots: vec![],
            companion_root: root.join("unused-companions").to_string_lossy().into_owned(),
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
        self.service.mutate(
            actor,
            OrchestrationMutationRequest {
                session_id: SESSION.into(),
                expected_revision,
                action,
            },
        )
    }

    fn operator(&self, action: OrchestrationAction) -> OrchestrationMutationResponse {
        self.apply(&Actor::Operator(OperatorOrigin::Browser), action)
            .unwrap()
    }

    fn root(&self) -> String {
        run_result(self.operator(OrchestrationAction::SupervisorStart {
            target: Some(target()),
            label: Some("Test supervisor".into()),
        }))
        .0
    }

    fn create_task(&self, root_id: &str, title: &str) -> Task {
        if self.run(root_id).stage == RunStage::Preparing {
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

    fn propose(&self, parent: &str, task: &Task, supersedes: Option<&str>) -> String {
        run_result(self.operator(proposal(parent, task, supersedes))).0
    }

    fn plan(&self, run_id: &str) -> PlanRecord {
        let text = format!("Initialize isolated workspace for {run_id}; do not execute yet");
        let plan = PlanRecord {
            plan_revision: plan_revision(&text).unwrap(),
            text,
            created_at: now(),
        };
        self.service
            .record_dispatch(
                run_id,
                DispatchUpdate::SetupPlanned {
                    setup: SetupSummary {
                        operation_id: None,
                        generation: None,
                        workspace_id: Some("test-space".into()),
                        checkout_path: self.root.join("checkout").to_string_lossy().into_owned(),
                        repository_id: None,
                        branch: None,
                        base: None,
                        ownership: None,
                        effects: vec!["Open isolated existing Space".into()],
                        warnings: Vec::new(),
                        project_workspace_id: None,
                    },
                    prepare_plan: plan.clone(),
                },
            )
            .unwrap();
        plan
    }

    fn prepare(&self, run_id: &str) -> PlanRecord {
        let plan = self.plan(run_id);
        self.operator(OrchestrationAction::GrantPrepare {
            run_id: run_id.into(),
            plan_revision: plan.plan_revision.clone(),
        });
        plan
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
            OrchestrationAction::RunBindSession {
                omp_session_id: caller.omp_session_id.clone().unwrap(),
            },
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
        Actor::Agent(AgentCaller {
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
            actual_agent_kind: Some("omp".into()),
            subagent_id: None,
            process: Some(NativeProcessIdentity {
                pid: 42,
                start_ticks: 123,
                kernel_boot_id: Some("kernel-one".into()),
            }),
        })
    }

    fn ready(&self, actor: &Actor) -> OrchestrationMutationResponse {
        self.apply(
            actor,
            report(
                ReportKind::Ready,
                None,
                Some("Implement the task, then report evidence"),
            ),
        )
        .unwrap()
    }

    fn working(&self, parent_run_id: &str) -> (Task, String, Actor) {
        let root_id = self.run(parent_run_id).root_id;
        let task = self.create_task(&root_id, "Worker task");
        let run_id = self.propose(parent_run_id, &task, None);
        self.prepare(&run_id);
        let actor = self.launch_and_bind(&run_id);
        self.ready(&actor);
        self.operator(OrchestrationAction::GrantExecute {
            run_id: run_id.clone(),
            plan_revision: self.run(&run_id).work_plan.unwrap().plan_revision,
            note: None,
        });
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
            origin: Some(GrantOrigin::Browser),
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

    fn queue_review(&self, run_id: &str) -> Run {
        self.operator(OrchestrationAction::ReconcileRun {
            run_id: run_id.into(),
            recovery: None,
        });
        let run = self.run(run_id);
        assert_eq!(
            run.dispatch.as_ref().unwrap().step,
            DispatchStep::LaunchIntent
        );
        assert!(run.dispatch.as_ref().unwrap().agent_started);
        run
    }

    fn assert_stale_review(&self, reviewed: &Run) {
        for step in [
            DispatchStep::Launched,
            DispatchStep::LaunchUnknown,
            DispatchStep::NeedsReview,
        ] {
            let revision = self.state().revision;
            let bytes = self.state_bytes();
            let receiver = self.service.revision.subscribe();
            assert_eq!(
                self.service
                    .record_launch_review(reviewed, step, Some(review_error()))
                    .unwrap(),
                revision
            );
            assert_eq!(self.state_bytes(), bytes, "Stale {step:?} must not save");
            assert!(
                !receiver.has_changed().unwrap(),
                "Stale proof must not notify"
            );
        }
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

fn proposal(parent: &str, task: &Task, supersedes: Option<&str>) -> OrchestrationAction {
    OrchestrationAction::RunPropose {
        task_id: task.task_id.clone(),
        parent_run_id: Some(parent.into()),
        label: None,
        target: target(),
        prepare_brief: "Inspect and initialize only; wait for Execute".into(),
        supersedes_run_id: supersedes.map(str::to_owned),
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
        to_run_id: None,
    }
}

fn run_result(response: OrchestrationMutationResponse) -> (String, u32) {
    match response.result {
        OrchestrationActionResult::Run { run_id, attempt } => (run_id, attempt),
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
    assert_eq!(fixture.run(&root).stage, RunStage::Preparing);
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
fn explicit_supersede_cannot_replace_a_sibling_subtree() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, branch, actor) = fixture.working(&root);
    let sibling_task = fixture.create_task(&root, "Sibling task");
    let sibling = fixture.propose(&root, &sibling_task, None);
    let before = fixture.state();
    assert_code(
        fixture.apply(&actor, proposal(&branch, &sibling_task, Some(&sibling))),
        "not_ancestor",
    );
    assert_eq!(fixture.state().revision, before.revision);
    assert_eq!(fixture.state().runs.len(), before.runs.len());
    assert_eq!(fixture.run(&sibling).stage, RunStage::Proposed);
    assert!(fixture.run(&sibling).close_reason.is_none());
    assert!(
        fixture
            .state()
            .messages
            .iter()
            .all(|message| message.kind != MessageKind::CancelRequest)
    );
}

#[test]
fn supersede_is_deferred_until_exact_prepare_grant() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, old, _) = fixture.working(&root);
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            proposal(&root, &task, None),
        ),
        "task_has_active_run",
    );
    let replacement = fixture.propose(&root, &task, Some(&old));
    assert_eq!(fixture.run(&replacement).attempt, 2);
    assert_eq!(
        fixture.run(&replacement).supersedes_run_id.as_deref(),
        Some(old.as_str())
    );
    assert_eq!(fixture.run(&old).stage, RunStage::Working);
    let plan = fixture.plan(&replacement);
    assert_eq!(fixture.run(&old).stage, RunStage::Working);
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::GrantPrepare {
                run_id: replacement.clone(),
                plan_revision: plan_revision(&"another plan").unwrap(),
            },
        ),
        "plan_changed",
    );
    assert_eq!(fixture.run(&old).stage, RunStage::Working);
    fixture.operator(OrchestrationAction::GrantPrepare {
        run_id: replacement.clone(),
        plan_revision: plan.plan_revision,
    });
    assert_eq!(fixture.run(&old).stage, RunStage::Closed);
    assert_eq!(
        fixture.run(&old).close_reason,
        Some(CloseReason::Superseded)
    );
    assert_eq!(fixture.run(&replacement).stage, RunStage::Preparing);
    let cancellations: Vec<_> = fixture
        .state()
        .messages
        .into_iter()
        .filter(|message| message.kind == MessageKind::CancelRequest && message.to_run_id == old)
        .collect();
    assert_eq!(cancellations.len(), 1);
    assert!(cancellations[0].text.contains(&replacement));
    assert!(!fixture.task(&root, &task.task_id).checked);
}

#[test]
fn prepare_and_execute_require_separate_exact_grants_and_keep_initialization_receipt() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Exact grants");
    let run_id = fixture.propose(&root, &task, None);
    let prepare = fixture.plan(&run_id);
    let root_actor = fixture.launch_and_bind(&root);
    let mut denied_actor = root_actor.clone();
    caller_mut(&mut denied_actor).actual_agent_kind = Some("shell".into());
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(
            &denied_actor,
            OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision.clone(),
            },
        ),
        "actor_forbidden",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: plan_revision(&"Changed initialization").unwrap(),
            },
        ),
        "plan_changed",
    );
    assert_eq!(fixture.run(&run_id).stage, RunStage::AwaitingPrepare);
    assert!(fixture.run(&run_id).grants.is_empty());
    fixture.operator(OrchestrationAction::GrantPrepare {
        run_id: run_id.clone(),
        plan_revision: prepare.plan_revision.clone(),
    });
    assert_eq!(fixture.run(&run_id).stage, RunStage::Preparing);
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::GrantExecute {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision.clone(),
                note: None,
            },
        ),
        "invalid_stage",
    );
    let actor = fixture.launch_and_bind(&run_id);
    fixture.ready(&actor);
    let ready = fixture.run(&run_id);
    let init = serde_json::to_value(ready.init_receipt.as_ref().unwrap()).unwrap();
    let work_plan = ready.work_plan.unwrap();
    assert_ne!(prepare.plan_revision, work_plan.plan_revision);
    assert_eq!(
        work_plan.plan_revision,
        plan_revision(&work_plan.text).unwrap()
    );
    assert_code(
        fixture.apply(
            &actor,
            OrchestrationAction::GrantExecute {
                run_id: run_id.clone(),
                plan_revision: work_plan.plan_revision.clone(),
                note: None,
            },
        ),
        "actor_forbidden",
    );
    assert_code(
        fixture.apply(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationAction::GrantExecute {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision.clone(),
                note: None,
            },
        ),
        "plan_changed",
    );
    assert_eq!(fixture.run(&run_id).stage, RunStage::Ready);
    assert_eq!(fixture.run(&run_id).grants.len(), 1);
    fixture.operator(OrchestrationAction::GrantExecute {
        run_id: run_id.clone(),
        plan_revision: work_plan.plan_revision.clone(),
        note: Some("Reviewed by operator".into()),
    });
    let working = fixture.run(&run_id);
    assert_eq!(working.stage, RunStage::Working);
    assert_eq!(
        serde_json::to_value(working.init_receipt.as_ref().unwrap()).unwrap(),
        init
    );
    assert_eq!(working.grants.len(), 2);
    assert_eq!(working.grants[0].scope, GrantScope::Prepare);
    assert_eq!(working.grants[0].plan_revision, prepare.plan_revision);
    assert_eq!(working.grants[1].scope, GrantScope::Execute);
    assert_eq!(working.grants[1].plan_revision, work_plan.plan_revision);
    assert_ne!(working.grants[0].grant_id, working.grants[1].grant_id);
    let briefs: Vec<_> = fixture
        .state()
        .messages
        .into_iter()
        .filter(|message| message.to_run_id == run_id && message.kind == MessageKind::WorkBrief)
        .collect();
    assert_eq!(briefs.len(), 1);
    assert!(briefs[0].text.starts_with(&work_plan.text));
    assert!(briefs[0].text.contains("Reviewed by operator"));
}

#[test]
fn successful_result_does_not_check_task_until_operator_accepts_exact_task_revision() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, actor) = fixture.working(&root);
    let init = serde_json::to_value(fixture.run(&run_id).init_receipt).unwrap();
    fixture
        .apply(
            &actor,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
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
    fixture.operator(OrchestrationAction::Accept {
        run_id: run_id.clone(),
        expected_task_revision: edited.task_revision,
    });
    assert!(fixture.task(&root, &task.task_id).checked);
    let accepted = fixture.run(&run_id);
    assert_eq!(accepted.stage, RunStage::Closed);
    assert_eq!(accepted.close_reason, Some(CloseReason::Accepted));
    assert_eq!(serde_json::to_value(accepted.init_receipt).unwrap(), init);
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
            .mutate(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationMutationRequest {
                    session_id: SESSION.into(),
                    expected_revision: Some(request.after_revision),
                    action: OrchestrationAction::SupervisorStart {
                        target: Some(target()),
                        label: Some("External service supervisor".into()),
                    },
                },
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
        fixture
            .apply(
                &actor,
                report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
            )
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
        fixture
            .apply(
                &actor,
                report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
            )
            .unwrap();
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
            fixture.pending_intent(&root, &fixture.task(&root, &task.task_id), &run_id);
        } else {
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
fn stale_incarnation_reports_remain_evidence_without_changing_live_stage() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Incarnation fences");
    let run_id = fixture.propose(&root, &task, None);
    fixture.prepare(&run_id);
    let actor = fixture.launch_and_bind(&run_id);
    let mut stale_callers = Vec::new();
    for fence in 0..6 {
        let mut stale = actor.clone();
        let caller = caller_mut(&mut stale);
        match fence {
            0 => caller.env_run.as_mut().unwrap().1 += 1,
            1 => caller.boot_id = Some("restarted-boot".into()),
            2 => caller.endpoint_identity = "replaced-endpoint".into(),
            3 => caller.terminal_id = Some("restarted-terminal".into()),
            4 => caller.native_session_id = Some("other-native".into()),
            5 => {
                caller.omp_session_id = Some("other-main".into());
                caller.main_omp_session_id = Some("other-main".into());
            }
            _ => unreachable!(),
        }
        stale_callers.push(stale);
    }
    for stale in &stale_callers {
        let response = fixture
            .apply(
                stale,
                report(ReportKind::Ready, None, Some("Stale work plan")),
            )
            .unwrap();
        assert!(matches!(
            response.result,
            OrchestrationActionResult::Message { stale: true, .. }
        ));
        let run = fixture.run(&run_id);
        assert_eq!(run.stage, RunStage::Initializing);
        assert!(run.init_receipt.is_none());
        assert!(run.work_plan.is_none());
        assert!(run.last_report.is_none());
    }
    fixture.ready(&actor);
    let init = serde_json::to_value(fixture.run(&run_id).init_receipt).unwrap();
    fixture.operator(OrchestrationAction::GrantExecute {
        run_id: run_id.clone(),
        plan_revision: fixture.run(&run_id).work_plan.unwrap().plan_revision,
        note: None,
    });
    for stale in &stale_callers {
        let response = fixture
            .apply(
                stale,
                report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
            )
            .unwrap();
        assert!(matches!(
            response.result,
            OrchestrationActionResult::Message { stale: true, .. }
        ));
        let run = fixture.run(&run_id);
        assert_eq!(run.stage, RunStage::Working);
        assert!(run.result.is_none());
        assert_eq!(serde_json::to_value(run.init_receipt).unwrap(), init);
    }
    assert!(!fixture.task(&root, &task.task_id).checked);
    let evidence: Vec<_> = fixture
        .state()
        .messages
        .into_iter()
        .filter(|message| message.stale)
        .collect();
    assert_eq!(evidence.len(), stale_callers.len() * 2);
    assert!(
        evidence
            .iter()
            .all(|message| message.to_run_id == root && message.report.is_some())
    );
}

#[test]
fn same_terminal_and_bound_native_move_preserves_identity_but_restart_does_not() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.launch_and_bind(&root);
    let original = fixture.run(&root);
    let mut moved = actor.clone();
    let caller = caller_mut(&mut moved);
    caller.workspace_id = "moved-space".into();
    caller.tab_id = "moved-tab".into();
    caller.pane_id = "moved-pane".into();
    caller.env_run = None;
    fixture
        .apply(
            &moved,
            OrchestrationAction::Annotate {
                run_id: root.clone(),
                text: "After move".into(),
            },
        )
        .unwrap();
    let after = fixture.run(&root);
    assert_eq!(after.run_id, original.run_id);
    assert_eq!(after.attempt, original.attempt);
    assert_eq!(after.stage, RunStage::Active);
    assert_eq!(after.bound_omp_session, original.bound_omp_session);
    let location = after.location.unwrap();
    assert_eq!(location.workspace_id, "moved-space");
    assert_eq!(location.tab_id, "moved-tab");
    assert_eq!(location.pane_id, "moved-pane");
    assert_eq!(fixture.state().runs.len(), 1);
    assert!(matches!(&after.annotations[0].by, ActorRef::Run { run_id } if run_id == &root));
    caller_mut(&mut moved).env_run = Some((root.clone(), original.attempt));
    for fence in 0..4 {
        let mut restarted = moved.clone();
        let caller = caller_mut(&mut restarted);
        match fence {
            0 => caller.boot_id = Some("boot-two".into()),
            1 => caller.terminal_id = Some("terminal-two".into()),
            2 => caller.endpoint_identity = "endpoint-two".into(),
            3 => {
                caller.native_session_id = Some("native-two".into());
                caller.omp_session_id = Some("native-two".into());
                caller.main_omp_session_id = Some("native-two".into());
            }
            _ => unreachable!(),
        }
        let revision = fixture.state().revision;
        assert_code(
            fixture.apply(
                &restarted,
                OrchestrationAction::Annotate {
                    run_id: root.clone(),
                    text: "Must not mutate the old run".into(),
                },
            ),
            "caller_mismatch",
        );
        assert_eq!(fixture.state().revision, revision);
        assert_eq!(fixture.run(&root).annotations.len(), 1);
        assert_eq!(
            fixture.run(&root).bound_omp_session,
            original.bound_omp_session
        );
    }
}

#[test]
fn moved_caller_uses_bound_omp_context_when_herdr_optional_ids_are_unavailable() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.launch_and_bind(&root);
    let mut moved = actor.clone();
    let caller = caller_mut(&mut moved);
    caller.workspace_id = "moved-space".into();
    caller.tab_id = "moved-tab".into();
    caller.pane_id = "moved-pane".into();
    caller.boot_id = None;
    caller.native_session_id = None;
    caller.env_run = None;
    fixture
        .apply(
            &moved,
            OrchestrationAction::Annotate {
                run_id: root.clone(),
                text: "Native OMP evidence remains bound".into(),
            },
        )
        .unwrap();
    assert_eq!(fixture.run(&root).location.unwrap().pane_id, "moved-pane");
    let mut missing_actual_session = moved.clone();
    caller_mut(&mut missing_actual_session).omp_session_id = None;
    assert!(
        fixture
            .apply(
                &missing_actual_session,
                OrchestrationAction::Annotate {
                    run_id: root,
                    text: "Cannot act without actual bound session".into(),
                }
            )
            .is_err()
    );
}

fn review_error() -> ErrorResponse {
    ErrorResponse {
        code: "launch_identity_conflict".into(),
        message: "Fresh runtime proof conflicts with the recorded launch".into(),
    }
}

// Compare every durable field, allowing only the documented review writes.
fn assert_review_fields_only(before: &Run, after: &Run) {
    let mut expected = serde_json::to_value(before).unwrap();
    let actual = serde_json::to_value(after).unwrap();
    expected["updated_at"] = actual["updated_at"].clone();
    for field in ["step", "error", "updated_at"] {
        expected["dispatch"][field] = actual["dispatch"][field].clone();
    }
    assert_eq!(actual, expected);
}

fn commit_review(
    fixture: &Fixture,
    reviewed: &Run,
    step: DispatchStep,
    error: Option<ErrorResponse>,
) {
    let before = fixture.state();
    let revision = fixture
        .service
        .record_launch_review(reviewed, step, error.clone())
        .unwrap();
    let after = fixture.state();
    assert_eq!(revision, before.revision + 1);
    assert_eq!(after.revision, revision);
    let index = run_index(&before, &reviewed.run_id).unwrap();
    assert_review_fields_only(&before.runs[index], &after.runs[index]);
    assert_eq!(after.runs[index].dispatch.as_ref().unwrap().step, step);
    assert_eq!(
        serde_json::to_value(&after.runs[index].dispatch.as_ref().unwrap().error).unwrap(),
        serde_json::to_value(error).unwrap()
    );
    let mut expected = serde_json::to_value(before).unwrap();
    let actual = serde_json::to_value(after).unwrap();
    expected["revision"] = actual["revision"].clone();
    expected["runs"][index] = actual["runs"][index].clone();
    let before_messages = expected["messages"].as_array().unwrap();
    let after_messages = actual["messages"].as_array().unwrap();
    assert_eq!(&after_messages[..before_messages.len()], before_messages);
    for message in after_messages.iter().skip(before_messages.len()) {
        let body: serde_json::Value = serde_json::from_str(message["text"].as_str().unwrap()).unwrap();
        assert!(matches!(body["event"].as_str(), Some("dispatch_failure" | "dispatch_recovered")));
        assert_eq!(body["run_id"], reviewed.run_id);
        assert_eq!(message["to_run_id"], reviewed.root_id);
    }
    expected["messages"] = actual["messages"].clone();
    assert_eq!(
        actual, expected,
        "Review must preserve other runs and existing inbox records"
    );
}

fn advance_worker(fixture: &Fixture, run_id: &str, actor: &Actor, stage: RunStage) {
    if matches!(
        stage,
        RunStage::Ready | RunStage::Working | RunStage::Reported
    ) {
        fixture.ready(actor);
    }
    if matches!(stage, RunStage::Working | RunStage::Reported) {
        fixture.operator(OrchestrationAction::GrantExecute {
            run_id: run_id.into(),
            plan_revision: fixture.run(run_id).work_plan.unwrap().plan_revision,
            note: Some("Exact reviewed execution grant".into()),
        });
    }
    if stage == RunStage::Reported {
        fixture
            .apply(
                actor,
                report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
            )
            .unwrap();
    }
}

fn launched_case(fixture: &Fixture, stage: RunStage) -> (String, Actor) {
    let root = fixture.root();
    if stage == RunStage::Active {
        fixture.plan(&root);
        let actor = fixture.launch_and_bind(&root);
        fixture
            .apply(
                &actor,
                report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
            )
            .unwrap();
        (root, actor)
    } else {
        let task = fixture.create_task(&root, "Review preserves canonical task");
        let run_id = fixture.propose(&root, &task, None);
        fixture.prepare(&run_id);
        let actor = fixture.launch_and_bind(&run_id);
        advance_worker(fixture, &run_id, &actor, stage);
        (run_id, actor)
    }
}

#[test]
fn launched_reconcile_preserves_lifecycle_receipts_grants_setup_identity_and_inbox() {
    for stage in [
        RunStage::Active,
        RunStage::Initializing,
        RunStage::Ready,
        RunStage::Working,
        RunStage::Reported,
    ] {
        for outcome in [
            DispatchStep::Launched,
            DispatchStep::LaunchUnknown,
            DispatchStep::NeedsReview,
        ] {
            let fixture = Fixture::new();
            let (run_id, actor) = launched_case(&fixture, stage);
            fixture
                .apply(
                    &actor,
                    OrchestrationAction::Annotate {
                        run_id: run_id.clone(),
                        text: "Retained review annotation".into(),
                    },
                )
                .unwrap();
            fixture.operator(OrchestrationAction::MessageSend {
                message_id: id(),
                to_run_id: run_id.clone(),
                kind: MessageKind::Instruction,
                text: "Retained unread instruction".into(),
                in_reply_to: None,
            });
            let before = fixture.run(&run_id);
            let messages = serde_json::to_value(fixture.state().messages).unwrap();
            let reviewed = fixture.queue_review(&run_id);
            assert_eq!(reviewed.stage, stage);
            assert_review_fields_only(&before, &reviewed);
            assert_eq!(
                serde_json::to_value(fixture.state().messages).unwrap(),
                messages
            );
            let error = (outcome != DispatchStep::Launched).then(review_error);
            commit_review(&fixture, &reviewed, outcome, error);
            assert_review_fields_only(&before, &fixture.run(&run_id));
            let retained = fixture.state().messages.into_iter().filter(|message| {
                !message.message_id.starts_with("dispatch-failure:")
                    && !message.message_id.starts_with("dispatch-recovered:")
            }).collect::<Vec<_>>();
            assert_eq!(serde_json::to_value(retained).unwrap(), messages);
            fixture.assert_stale_review(&reviewed);
            if outcome != DispatchStep::Launched {
                let uncertain = fixture.run(&run_id);
                let queued_again = fixture.queue_review(&run_id);
                assert_review_fields_only(&uncertain, &queued_again);
                assert!(queued_again.dispatch.unwrap().agent_started);
            }
            let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
            // Drop the shared named-lock guard before reading through the other service.
            let reopened_state = {
                let locked = reopened.store.lock().unwrap();
                locked.read().unwrap()
            };
            assert_eq!(
                serde_json::to_value(reopened_state).unwrap(),
                serde_json::to_value(fixture.state()).unwrap()
            );
        }
    }
}

#[test]
fn launched_reconcile_rejects_setup_recovery_and_closed_without_save_and_bad_marker_fails_closed() {
    let fixture = Fixture::new();
    let (run_id, _) = launched_case(&fixture, RunStage::Active);
    for recovery in [
        cockpit_protocol::projects::WorkspaceRecoveryAction::AcceptExistingWorktree,
        cockpit_protocol::projects::WorkspaceRecoveryAction::RetryEnvironment,
    ] {
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.apply(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationAction::ReconcileRun {
                    run_id: run_id.clone(),
                    recovery: Some(recovery),
                },
            ),
            "invalid_stage",
        );
        assert_eq!(fixture.state_bytes(), bytes);
    }
    fixture.seed_run(&run_id, |run| {
        run.dispatch.as_mut().unwrap().agent_started = false;
    });
    let before = fixture.run(&run_id);
    let messages = serde_json::to_value(fixture.state().messages).unwrap();
    fixture.operator(OrchestrationAction::ReconcileRun {
        run_id: run_id.clone(),
        recovery: None,
    });
    let after = fixture.run(&run_id);
    assert_eq!(
        after.dispatch.as_ref().unwrap().step,
        DispatchStep::NeedsReview
    );
    assert_eq!(
        after
            .dispatch
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .code,
        "launch_receipt_inconsistent"
    );
    assert!(!after.dispatch.as_ref().unwrap().agent_started);
    assert_review_fields_only(&before, &after);
    assert_eq!(
        serde_json::to_value(fixture.state().messages).unwrap(),
        messages
    );
    fixture.operator(OrchestrationAction::CancelRun {
        run_id: run_id.clone(),
    });
    for step in [
        DispatchStep::Launched,
        DispatchStep::LaunchUnknown,
        DispatchStep::NeedsReview,
    ] {
        fixture.seed_run(&run_id, |run| run.dispatch.as_mut().unwrap().step = step);
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.apply(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationAction::ReconcileRun {
                    run_id: run_id.clone(),
                    recovery: None,
                },
            ),
            "invalid_stage",
        );
        assert_eq!(fixture.state_bytes(), bytes);
    }
}

#[test]
fn launch_review_accepts_only_terminal_outcomes_on_current_proven_queue() {
    let fixture = Fixture::new();
    let (run_id, _) = launched_case(&fixture, RunStage::Active);
    fixture.assert_stale_review(&fixture.run(&run_id));
    let reviewed = fixture.queue_review(&run_id);
    for step in [
        DispatchStep::Planning,
        DispatchStep::PlanFailed,
        DispatchStep::SetupPending,
        DispatchStep::SetupRunning,
        DispatchStep::SetupUnknown,
        DispatchStep::LaunchIntent,
    ] {
        let bytes = fixture.state_bytes();
        assert!(
            fixture
                .service
                .record_launch_review(&reviewed, step, None)
                .is_err()
        );
        assert_eq!(fixture.state_bytes(), bytes);
    }
    let mut unproven = reviewed.clone();
    unproven.dispatch.as_mut().unwrap().agent_started = false;
    fixture.assert_stale_review(&unproven);
    commit_review(&fixture, &reviewed, DispatchStep::Launched, None);
}

#[test]
fn launch_review_lifecycle_races_preserve_current_ready_execute_worker_and_active_root_results() {
    for race in 0..4 {
        let fixture = Fixture::new();
        let stage = match race {
            0 => RunStage::Initializing,
            1 => RunStage::Ready,
            2 => RunStage::Working,
            _ => RunStage::Active,
        };
        let (run_id, actor) = launched_case(&fixture, stage);
        let reviewed = fixture.queue_review(&run_id);
        match race {
            0 => {
                fixture.ready(&actor);
            }
            1 => {
                fixture.operator(OrchestrationAction::GrantExecute {
                    run_id: run_id.clone(),
                    plan_revision: fixture.run(&run_id).work_plan.unwrap().plan_revision,
                    note: Some("Granted while runtime proof was outstanding".into()),
                });
            }
            _ => {
                fixture
                    .apply(
                        &actor,
                        report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
                    )
                    .unwrap();
            }
        }
        let current = fixture.run(&run_id);
        assert_eq!(
            current.stage,
            [
                RunStage::Ready,
                RunStage::Working,
                RunStage::Reported,
                RunStage::Active
            ][race]
        );
        if race == 3 {
            assert_ne!(
                current.result.as_ref().unwrap().message_id,
                reviewed.result.as_ref().unwrap().message_id
            );
        }
        commit_review(&fixture, &reviewed, DispatchStep::Launched, None);
        assert_review_fields_only(&current, &fixture.run(&run_id));
    }
}

#[test]
fn late_launch_review_is_discarded_after_cancel_accept_reconcile_retry_bind_or_move() {
    for race in 0..6 {
        let fixture = Fixture::new();
        let stage = if race == 1 {
            RunStage::Reported
        } else {
            RunStage::Active
        };
        let (run_id, actor) = launched_case(&fixture, stage);
        if race == 4 {
            // Model a proven launch whose extension bind has not committed yet.
            fixture.seed_run(&run_id, |run| run.bound_omp_session = None);
        }
        fixture.queue_review(&run_id);
        if race == 2 {
            // A persisted older request token makes replacement deterministic,
            // without sleeping or relying on clock resolution.
            fixture.seed_run(&run_id, |run| {
                run.dispatch.as_mut().unwrap().updated_at = "2000-01-01T00:00:00Z".into();
            });
        }
        let reviewed = fixture.run(&run_id);
        match race {
            0 => {
                fixture.operator(OrchestrationAction::CancelRun {
                    run_id: run_id.clone(),
                });
            }
            1 => {
                let run = fixture.run(&run_id);
                let task = fixture.task(&run.root_id, run.task_id.as_deref().unwrap());
                fixture.operator(OrchestrationAction::Accept {
                    run_id: run_id.clone(),
                    expected_task_revision: task.task_revision,
                });
            }
            2 => {
                let newer = fixture.queue_review(&run_id);
                assert_ne!(
                    newer.dispatch.as_ref().unwrap().updated_at,
                    reviewed.dispatch.as_ref().unwrap().updated_at
                );
            }
            3 => {
                commit_review(
                    &fixture,
                    &reviewed,
                    DispatchStep::NeedsReview,
                    Some(review_error()),
                );
                fixture.operator(OrchestrationAction::RetryLaunch {
                    run_id: run_id.clone(),
                });
            }
            4 => {
                let Actor::Agent(caller) = &actor else {
                    unreachable!();
                };
                fixture
                    .apply(
                        &actor,
                        OrchestrationAction::RunBindSession {
                            omp_session_id: caller.omp_session_id.clone().unwrap(),
                        },
                    )
                    .unwrap();
            }
            5 => {
                let mut moved = actor.clone();
                let caller = caller_mut(&mut moved);
                caller.workspace_id = "relocated-space".into();
                caller.tab_id = "relocated-tab".into();
                caller.pane_id = "relocated-pane".into();
                fixture
                    .apply(
                        &moved,
                        OrchestrationAction::Annotate {
                            run_id: run_id.clone(),
                            text: "Actual same-terminal relocation while review waits".into(),
                        },
                    )
                    .unwrap();
                assert_eq!(
                    fixture.run(&run_id).location.unwrap().pane_id,
                    "relocated-pane"
                );
            }
            _ => unreachable!(),
        }
        fixture.assert_stale_review(&reviewed);
    }
}

#[test]
fn launch_review_guards_every_incarnation_request_and_location_field() {
    let fixture = Fixture::new();
    let (run_id, _) = launched_case(&fixture, RunStage::Active);
    let reviewed = fixture.queue_review(&run_id);
    // Alter the captured proof, not the durable queue: each mismatch independently
    // must discard proof without depending on a second changed guard.
    for fence in 0..21 {
        let mut stale = reviewed.clone();
        match fence {
            0 => stale.run_id = id(),
            1 => stale.session_id = "other-session".into(),
            2 => stale.root_id = id(),
            3 => stale.attempt += 1,
            4 => stale.dispatch.as_mut().unwrap().launch_attempt += 1,
            5 => stale.dispatch.as_mut().unwrap().launch_tag = Some("other-tag".into()),
            6 => stale.dispatch.as_mut().unwrap().endpoint_identity = Some("other-endpoint".into()),
            7 => stale.dispatch.as_mut().unwrap().agent_started = false,
            8 => stale.dispatch.as_mut().unwrap().step = DispatchStep::Launched,
            9 => stale.dispatch.as_mut().unwrap().updated_at = "obsolete-request-token".into(),
            10 => stale.bound_omp_session = Some("other-binding".into()),
            11 => stale.location = None,
            12 => stale.location.as_mut().unwrap().boot_id = Some("other-boot".into()),
            13 => stale.location.as_mut().unwrap().terminal_id = Some("other-terminal".into()),
            14 => stale.location.as_mut().unwrap().native_session_id = Some("other-native".into()),
            15 => stale.location.as_mut().unwrap().endpoint_identity = "other-endpoint".into(),
            16 => stale.location.as_mut().unwrap().session_id = "other-session".into(),
            17 => stale.location.as_mut().unwrap().workspace_id = "other-space".into(),
            18 => stale.location.as_mut().unwrap().tab_id = "other-tab".into(),
            19 => stale.location.as_mut().unwrap().pane_id = "other-pane".into(),
            20 => stale.location.as_mut().unwrap().launch_tag = "other-tag".into(),
            _ => unreachable!(),
        }
        fixture.assert_stale_review(&stale);
    }
    let mut missing_dispatch = reviewed.clone();
    missing_dispatch.dispatch = None;
    fixture.assert_stale_review(&missing_dispatch);
    fixture.seed_run(&run_id, |run| {
        run.dispatch.as_mut().unwrap().agent_started = false
    });
    fixture.assert_stale_review(&reviewed);
}

#[test]
fn busy_inbox_and_unrelated_revisions_do_not_starve_launch_review() {
    let fixture = Fixture::new();
    let (run_id, actor) = launched_case(&fixture, RunStage::Active);
    let reviewed = fixture.queue_review(&run_id);
    let other = fixture.root();
    for index in 0..3 {
        fixture.operator(OrchestrationAction::MessageSend {
            message_id: id(),
            to_run_id: run_id.clone(),
            kind: MessageKind::Instruction,
            text: format!("Inbox traffic while reviewing {index}"),
            in_reply_to: None,
        });
        fixture.operator(OrchestrationAction::Annotate {
            run_id: other.clone(),
            text: format!("Unrelated durable revision {index}"),
        });
    }
    fixture
        .apply(
            &actor,
            OrchestrationAction::InboxPull {
                after_seq: 0,
                limit: 100,
            },
        )
        .unwrap();
    assert!(
        fixture
            .state()
            .messages
            .iter()
            .any(|message| { message.to_run_id == run_id && message.stage == DeliveryStage::Read })
    );
    let current = fixture.run(&run_id);
    commit_review(&fixture, &reviewed, DispatchStep::Launched, None);
    assert_review_fields_only(&current, &fixture.run(&run_id));
}

#[test]
fn uncertain_review_requires_explicit_retry_and_retains_setup_receipts_grants_and_unacked_inbox() {
    for outcome in [DispatchStep::LaunchUnknown, DispatchStep::NeedsReview] {
        let fixture = Fixture::new();
        let (run_id, old_actor) = launched_case(&fixture, RunStage::Reported);
        fixture.seed_run(&run_id, |run| {
            let setup = run.setup.as_mut().unwrap();
            setup.operation_id = Some("retained-completed-setup".into());
            setup.generation = Some(3);
            setup.repository_id = Some("retained-repository".into());
            setup.branch = Some("retained-checkout-branch".into());
            setup.base = Some("retained-base-revision".into());
            setup.warnings.push("Retained setup warning".into());
        });
        fixture.operator(OrchestrationAction::MessageSend {
            message_id: id(),
            to_run_id: run_id.clone(),
            kind: MessageKind::Instruction,
            text: "Replay this original unread instruction after explicit retry".into(),
            in_reply_to: None,
        });
        for queued in [false, true] {
            if queued {
                fixture.queue_review(&run_id);
            }
            let bytes = fixture.state_bytes();
            assert_code(
                fixture.apply(
                    &Actor::Operator(OperatorOrigin::Browser),
                    OrchestrationAction::RetryLaunch {
                        run_id: run_id.clone(),
                    },
                ),
                "invalid_stage",
            );
            assert_eq!(fixture.state_bytes(), bytes);
        }
        let reviewed = fixture.run(&run_id);
        commit_review(&fixture, &reviewed, outcome, Some(review_error()));
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.apply(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationAction::RetryLaunch {
                    run_id: run_id.clone(),
                },
            ),
            "invalid_stage",
        );
        assert_eq!(
            fixture.state_bytes(),
            bytes,
            "Unreviewed Result must survive uncertain startup"
        );
        fixture.operator(OrchestrationAction::SendBack {
            run_id: run_id.clone(),
            text: "Reviewed Result; explicitly initialize a new launch before further work".into(),
        });
        let original = fixture.run(&run_id);
        let messages = serde_json::to_value(fixture.state().messages).unwrap();
        fixture.operator(OrchestrationAction::RetryLaunch {
            run_id: run_id.clone(),
        });
        let retried = fixture.run(&run_id);
        assert_eq!(retried.stage, RunStage::Preparing);
        assert!(retried.location.is_none());
        assert!(retried.bound_omp_session.is_none());
        assert!(original.bound_omp_process.is_some());
        assert!(original.launch_shell_identity.is_some());
        assert!(retried.bound_omp_process.is_none());
        assert!(retried.launch_shell_identity.is_none());
        // Only accepted Closed runs obtain retirement authority; Closed runs
        // cannot retry. A permitted uncertain-launch retry has none.
        assert!(retried.retirement.is_none());
        let dispatch = retried.dispatch.as_ref().unwrap();
        assert_eq!(dispatch.step, DispatchStep::SetupPending);
        assert_eq!(
            dispatch.launch_attempt,
            original.dispatch.as_ref().unwrap().launch_attempt + 1
        );
        assert!(!dispatch.agent_started);
        assert!(dispatch.launch_tag.is_none());
        assert!(dispatch.endpoint_identity.is_none());
        assert_eq!(retried.task_id, original.task_id);
        assert_eq!(retried.prepare_brief, original.prepare_brief);
        assert_eq!(
            serde_json::to_value(&retried.setup).unwrap(),
            serde_json::to_value(&original.setup).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&retried.grants).unwrap(),
            serde_json::to_value(&original.grants).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&retried.work_plan).unwrap(),
            serde_json::to_value(&original.work_plan).unwrap()
        );
        let mut expected_messages: Vec<Message> = serde_json::from_value(messages.clone()).unwrap();
        for message in expected_messages.iter_mut().filter(|message| {
            message.to_run_id == run_id
                && matches!(
                    message.kind,
                    MessageKind::WorkBrief
                        | MessageKind::PrepareBrief
                        | MessageKind::SupervisorBrief
                )
        }) {
            message.stale = true;
        }
        assert_eq!(
            serde_json::to_value(fixture.state().messages).unwrap(),
            serde_json::to_value(&expected_messages).unwrap()
        );
        fixture.assert_stale_review(&reviewed);
        let new_actor = fixture.launch_and_bind(&run_id);
        let settled = fixture.run(&run_id);
        assert_eq!(settled.stage, RunStage::Initializing);
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.apply(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationAction::GrantExecute {
                    run_id: run_id.clone(),
                    plan_revision: original.work_plan.as_ref().unwrap().plan_revision.clone(),
                    note: None,
                },
            ),
            "invalid_stage",
        );
        assert_eq!(
            fixture.state_bytes(),
            bytes,
            "Historical execute grants must not resume work"
        );
        assert_ne!(settled.bound_omp_session, original.bound_omp_session);
        assert_eq!(settled.attempt, original.attempt);
        assert_eq!(settled.root_id, original.root_id);
        assert_eq!(
            serde_json::to_value(&settled.setup).unwrap(),
            serde_json::to_value(&original.setup).unwrap()
        );
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.apply(
                &old_actor,
                OrchestrationAction::Annotate {
                    run_id: run_id.clone(),
                    text: "Old session must not mutate the retry".into(),
                },
            ),
            "caller_mismatch",
        );
        assert_eq!(fixture.state_bytes(), bytes);
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.apply(&old_actor, OrchestrationAction::InboxAck { through_seq: 1 }),
            "caller_mismatch",
        );
        assert_eq!(fixture.state_bytes(), bytes);
        let prior_messages = fixture.state().messages;
        let response = fixture
            .apply(
                &old_actor,
                report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
            )
            .unwrap();
        assert!(matches!(
            response.result,
            OrchestrationActionResult::Message { stale: true, .. }
        ));
        let evidence = fixture.state().messages;
        assert_eq!(evidence.len(), prior_messages.len() + 1);
        assert_eq!(
            serde_json::to_value(&evidence[..prior_messages.len()]).unwrap(),
            serde_json::to_value(&prior_messages).unwrap()
        );
        let appended = evidence.last().unwrap();
        assert!(appended.stale);
        assert_eq!(appended.to_run_id, original.root_id);
        assert_eq!(appended.kind, MessageKind::Report);
        assert_eq!(appended.report.as_ref().unwrap().kind, ReportKind::Result);
        assert_eq!(
            serde_json::to_value(fixture.run(&run_id)).unwrap(),
            serde_json::to_value(&settled).unwrap()
        );
        let response = fixture
            .apply(
                &new_actor,
                OrchestrationAction::InboxPull {
                    after_seq: 0,
                    limit: 100,
                },
            )
            .unwrap();
        let OrchestrationActionResult::Inbox {
            messages: pulled, ..
        } = response.result
        else {
            panic!("Expected the new bound session to pull the retained inbox");
        };
        let original_messages: Vec<Message> = serde_json::from_value(messages).unwrap();
        for old in original_messages
            .iter()
            .filter(|message| message.to_run_id == run_id)
        {
            let current = pulled
                .iter()
                .find(|message| message.message_id == old.message_id)
                .unwrap();
            assert_eq!(current.seq, old.seq);
            assert_eq!(current.text, old.text);
            if matches!(
                old.kind,
                MessageKind::WorkBrief | MessageKind::PrepareBrief | MessageKind::SupervisorBrief
            ) {
                assert!(
                    current.stale,
                    "Old launch briefs are history, not executable mail"
                );
            } else {
                assert_eq!(current.stale, old.stale);
            }
            assert_eq!(current.stage, DeliveryStage::Read);
            assert!(current.acked_at.is_none());
        }
        assert!(
            !fixture
                .task(&original.root_id, original.task_id.as_deref().unwrap())
                .checked
        );
    }
}

#[test]
fn first_launch_proof_preserves_fast_same_launch_progress_and_repeated_proof_is_no_write() {
    for stage in [
        RunStage::Initializing,
        RunStage::Ready,
        RunStage::Working,
        RunStage::Reported,
        RunStage::Active,
    ] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let run_id = if stage == RunStage::Active {
            fixture.plan(&root);
            root
        } else {
            let task = fixture.create_task(&root, "Fast same-launch progress");
            let run_id = fixture.propose(&root, &task, None);
            fixture.prepare(&run_id);
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
                OrchestrationAction::RunBindSession {
                    omp_session_id: caller.omp_session_id.clone().unwrap(),
                },
            )
            .unwrap();
        let reviewed = fixture.run(&run_id);
        // Model lifecycle advancement during the runtime proof read. Keep the
        // captured Preparing incarnation, and exercise Ready/Execute/Result
        // through actual service mutations rather than forging their receipts.
        fixture.seed_run(&run_id, |run| {
            run.stage = if stage == RunStage::Active {
                RunStage::Active
            } else {
                RunStage::Initializing
            };
        });
        if stage == RunStage::Active {
            fixture
                .apply(
                    &actor,
                    report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
                )
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
        let mut expected = serde_json::to_value(&before).unwrap();
        let actual = serde_json::to_value(&after).unwrap();
        expected["updated_at"] = actual["updated_at"].clone();
        expected["dispatch"] = actual["dispatch"].clone();
        assert_eq!(actual, expected);
        assert!(after.dispatch.as_ref().unwrap().agent_started);
        assert_eq!(
            after.dispatch.as_ref().unwrap().step,
            DispatchStep::Launched
        );
        let kind = if stage == RunStage::Active {
            MessageKind::SupervisorBrief
        } else {
            MessageKind::PrepareBrief
        };
        let messages = fixture.state().messages;
        let new_briefs: Vec<_> = messages
            .iter()
            .filter(|message| message.to_run_id == run_id && message.kind == kind)
            .collect();
        assert_eq!(new_briefs.len(), 1);
        let retained: Vec<_> = messages
            .into_iter()
            .filter(|message| !(message.to_run_id == run_id && message.kind == kind))
            .collect();
        assert_eq!(serde_json::to_value(retained).unwrap(), original_messages);
        // Even while a review is queued, a repeated proven start cannot bypass
        // runtime proof or reissue/reinitialize launch work.
        for queued in [false, true] {
            if queued {
                fixture.queue_review(&run_id);
            }
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
            let run_id = fixture.propose(&root, &task, None);
            fixture.prepare(&run_id);
            run_id
        } else {
            root
        };
        let actor = fixture.launch_tab(&run_id);
        let unbound = fixture.run(&run_id);
        assert_eq!(unbound.stage, RunStage::Preparing);
        assert!(unbound.bound_omp_session.is_none());
        assert_proof_discarded(&fixture, &unbound);
        let Actor::Agent(caller) = &actor else {
            unreachable!()
        };
        fixture
            .apply(
                &actor,
                OrchestrationAction::RunBindSession {
                    omp_session_id: caller.omp_session_id.clone().unwrap(),
                },
            )
            .unwrap();
        assert_eq!(fixture.run(&run_id).stage, RunStage::Preparing);
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
        assert_eq!(pending.stage, RunStage::Preparing);
        assert_eq!(pending.bound_omp_session, caller.omp_session_id);
        assert!(fixture.state().messages.iter().all(|message| {
            message.to_run_id != run_id
                || !matches!(
                    message.kind,
                    MessageKind::SupervisorBrief | MessageKind::PrepareBrief
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
                RunStage::Initializing
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
            MessageKind::PrepareBrief
        } else {
            MessageKind::SupervisorBrief
        };
        let messages = fixture.state().messages;
        let briefs: Vec<_> = messages
            .iter()
            .filter(|message| message.to_run_id == run_id && message.kind == kind)
            .collect();
        assert_eq!(briefs.len(), 1);
        assert_eq!(
            briefs[0].message_id,
            format!(
                "brief:{run_id}:launch-{}-{}:launch",
                proven.attempt,
                proven.dispatch.as_ref().unwrap().launch_attempt,
            )
        );
        assert_eq!(briefs[0].text, proven.prepare_brief);
        assert_proof_discarded(&fixture, &proven);
    }
}

#[test]
fn launch_proof_fences_target_task_native_binding_and_every_receipt_field() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Exact launch proof");
    let run_id = fixture.propose(&root, &task, None);
    fixture.prepare(&run_id);
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
            OrchestrationAction::RunBindSession {
                omp_session_id: caller.omp_session_id.clone().unwrap(),
            },
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
            7 => stale.bound_omp_session = None,
            8 => stale.bound_omp_session = Some(String::new()),
            9 => stale.bound_omp_session = Some("other-native-main".into()),
            10 => stale.location = None,
            11 => stale.location.as_mut().unwrap().boot_id = Some("different-boot".into()),
            12 => stale.location.as_mut().unwrap().terminal_id = Some("different-terminal".into()),
            13 => {
                stale.location.as_mut().unwrap().native_session_id = Some("different-native".into())
            }
            14 => stale.location.as_mut().unwrap().endpoint_identity = "different-endpoint".into(),
            15 => stale.location.as_mut().unwrap().session_id = "different-session".into(),
            16 => stale.location.as_mut().unwrap().workspace_id = "different-space".into(),
            17 => stale.location.as_mut().unwrap().tab_id = "different-tab".into(),
            18 => stale.location.as_mut().unwrap().pane_id = "different-pane".into(),
            19 => stale.location.as_mut().unwrap().launch_tag = "different-tag".into(),
            _ => unreachable!(),
        }
        assert_proof_discarded(&fixture, &stale);
    }
    // Even matching captured/current corrupt receipts are not valid proof.
    for fence in 0..5 {
        fixture.seed_run(&run_id, |run| {
            *run = reviewed.clone();
            match fence {
                0 => run.bound_omp_session = Some(String::new()),
                1 => {
                    run.location.as_mut().unwrap().native_session_id =
                        Some("conflicting-native".into())
                }
                2 => run.location.as_mut().unwrap().launch_tag.clear(),
                3 => {
                    run.dispatch.as_mut().unwrap().endpoint_identity = Some("wrong-endpoint".into())
                }
                4 => run.location.as_mut().unwrap().session_id = "wrong-session".into(),
                _ => unreachable!(),
            }
        });
        assert_proof_discarded(&fixture, &fixture.run(&run_id));
    }
    fixture.seed_run(&run_id, |run| {
        *run = reviewed;
        // Herdr's optional native ID is not a required launch gate.
        run.location.as_mut().unwrap().native_session_id = None;
    });
    fixture
        .service
        .record_launch_verified(&fixture.run(&run_id))
        .unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Initializing);
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
                    OrchestrationAction::RunBindSession {
                        omp_session_id: caller.omp_session_id.clone().unwrap(),
                    },
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
                fixture.operator(OrchestrationAction::RetryLaunch {
                    run_id: root.clone(),
                });
                assert_eq!(
                    fixture.run(&root).dispatch.unwrap().launch_attempt,
                    reviewed.dispatch.as_ref().unwrap().launch_attempt + 1
                );
            }
            2 => {
                fixture
                    .apply(
                        &actor,
                        OrchestrationAction::RunBindSession {
                            omp_session_id: caller.omp_session_id.clone().unwrap(),
                        },
                    )
                    .unwrap();
            }
            3 => {
                let mut moved = actor.clone();
                let moved_caller = caller_mut(&mut moved);
                moved_caller.workspace_id = "moved-space".into();
                moved_caller.tab_id = "moved-tab".into();
                moved_caller.pane_id = "moved-pane".into();
                fixture
                    .apply(
                        &moved,
                        OrchestrationAction::Annotate {
                            run_id: root.clone(),
                            text: "Same terminal moved while proof waits".into(),
                        },
                    )
                    .unwrap();
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
    let answer = OrchestrationAction::MessageSend {
        message_id: id(),
        to_run_id: run_id.clone(),
        kind: MessageKind::Answer,
        text: "Use the approved choice".into(),
        in_reply_to: Some(q1.clone()),
    };
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
        assert_code(fixture.apply(&supervisor, changed), "message_id_conflict");
        assert_eq!(fixture.state_bytes(), bytes);
    }
    let mut obsolete = supervisor.clone();
    caller_mut(&mut obsolete).omp_session_id = Some("obsolete-main".into());
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&obsolete, answer), "session_mismatch");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn bound_top_root_manages_exact_prepare_execute_answer_sendback_accept_and_cancel() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "User chat task requires no new operator grant");
    let run_id = fixture.propose(&root, &task, None);
    let prepare = fixture.plan(&run_id);
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(
            &supervisor,
            OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: plan_revision(&"Unreviewed setup").unwrap(),
            },
        ),
        "plan_changed",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision.clone(),
            },
        )
        .unwrap();
    let worker = fixture.launch_and_bind(&run_id);
    fixture.ready(&worker);
    let ready = fixture.run(&run_id);
    let work_plan = ready.work_plan.as_ref().unwrap();
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(
            &supervisor,
            OrchestrationAction::GrantExecute {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision.clone(),
                note: None,
            },
        ),
        "plan_changed",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::GrantExecute {
                run_id: run_id.clone(),
                plan_revision: work_plan.plan_revision.clone(),
                note: Some("Exact Ready receipt reviewed by the root".into()),
            },
        )
        .unwrap();
    let working = fixture.run(&run_id);
    assert_eq!(
        serde_json::to_value(&working.init_receipt).unwrap(),
        serde_json::to_value(&ready.init_receipt).unwrap()
    );
    let native = fixture.run(&root).bound_omp_session.unwrap();
    assert_eq!(working.grants.len(), 2);
    for grant in &working.grants {
        assert_eq!(grant.origin, GrantOrigin::Supervisor);
        assert_eq!(grant.supervisor_run_id.as_deref(), Some(root.as_str()));
        assert_eq!(grant.omp_session_id.as_deref(), Some(native.as_str()));
    }
    assert_eq!(working.grants[0].scope, GrantScope::Prepare);
    assert_eq!(working.grants[0].plan_revision, prepare.plan_revision);
    assert_eq!(working.grants[1].scope, GrantScope::Execute);
    assert_eq!(working.grants[1].plan_revision, work_plan.plan_revision);
    fixture
        .apply(&worker, report(ReportKind::NeedsInput, None, None))
        .unwrap();
    let question_id = fixture.run(&run_id).last_report.unwrap().message_id;
    let answer_id = id();
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::MessageSend {
                message_id: answer_id.clone(),
                to_run_id: run_id.clone(),
                kind: MessageKind::Answer,
                text: "Use the established task scope; no additional decision is missing".into(),
                in_reply_to: Some(question_id.clone()),
            },
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
    fixture
        .apply(
            &worker,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
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
    let question = projection::question_status(run, &inbox).unwrap();
    assert_eq!(question.question_message_id, sendback_question);
    assert!(matches!(question.receipt, QuestionReceipt::Unresolved));
    fixture
        .apply(
            &worker,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
        .unwrap();
    let result_id = fixture.run(&run_id).result.unwrap().message_id;
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
    let survivor_before = serde_json::to_value(fixture.run(&survivor)).unwrap();
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
    assert_eq!(
        serde_json::to_value(fixture.run(&survivor)).unwrap(),
        survivor_before
    );
    let cancel = fixture
        .state()
        .messages
        .into_iter()
        .find(|message| {
            message.to_run_id == cancelled && message.kind == MessageKind::CancelRequest
        })
        .unwrap();
    assert_agent_decision(&cancel, &root);
    assert!(cancel.text.contains("Tracking is closed"));
    assert!(
        cancel
            .text
            .contains("does not guarantee process termination")
    );
}

#[test]
fn management_never_inherits_authority_from_worker_subagent_stale_or_non_omp_root() {
    for operation in 0..8 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let supervisor = fixture.launch_and_bind(&root);
        let task = fixture.create_task(&root, "Authority boundary");
        let run_id = fixture.propose(&root, &task, None);
        let prepare = fixture.plan(&run_id);
        let worker = if operation == 0 {
            None
        } else {
            fixture.operator(OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision.clone(),
            });
            let actor = fixture.launch_and_bind(&run_id);
            fixture.ready(&actor);
            if operation != 1 {
                fixture.operator(OrchestrationAction::GrantExecute {
                    run_id: run_id.clone(),
                    plan_revision: fixture.run(&run_id).work_plan.unwrap().plan_revision,
                    note: None,
                });
            }
            if matches!(operation, 2 | 3) {
                fixture
                    .apply(
                        &actor,
                        report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
                    )
                    .unwrap();
            }
            Some(actor)
        };
        let action = match operation {
            0 => OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision,
            },
            1 => OrchestrationAction::GrantExecute {
                run_id: run_id.clone(),
                plan_revision: fixture.run(&run_id).work_plan.unwrap().plan_revision,
                note: None,
            },
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
            5 => OrchestrationAction::MessageSend {
                message_id: id(),
                to_run_id: run_id.clone(),
                kind: MessageKind::Answer,
                text: "Answer".into(),
                in_reply_to: None,
            },
            6 => OrchestrationAction::ReconcileRun {
                run_id: run_id.clone(),
                recovery: None,
            },
            7 => OrchestrationAction::RetryLaunch {
                run_id: run_id.clone(),
            },
            _ => unreachable!(),
        };
        let other_root = fixture.root();
        let outsider = fixture.launch_and_bind(&other_root);
        let (_, branch_id, branch_actor) = fixture.working(&root);
        let mut inherited_env = branch_actor.clone();
        caller_mut(&mut inherited_env).env_run = Some((root.clone(), 1));
        let mut denied = vec![outsider, branch_actor.clone(), inherited_env];
        if let Some(worker) = worker {
            denied.push(worker);
        }
        for fence in 0..10 {
            let mut actor = supervisor.clone();
            let caller = caller_mut(&mut actor);
            match fence {
                0 => caller.actual_agent_kind = None,
                1 => caller.actual_agent_kind = Some("shell".into()),
                2 => caller.actual_agent_kind = Some("OMP".into()),
                3 => caller.agent_kind = None,
                4 => {
                    caller.agent_kind = Some(AgentKind::Subagent);
                    caller.subagent_id = Some("internal-child".into());
                    caller.omp_session_id = Some("internal-child-native".into());
                }
                5 => caller.omp_session_id = Some("obsolete-main".into()),
                6 => caller.native_session_id = Some("other-occupant".into()),
                7 => caller.endpoint_identity = "other-endpoint".into(),
                8 => caller.terminal_id = Some("new-terminal".into()),
                9 => caller.env_run.as_mut().unwrap().1 += 1,
                _ => unreachable!(),
            }
            denied.push(actor);
        }
        let mut branch_subagent = branch_actor;
        let caller = caller_mut(&mut branch_subagent);
        caller.agent_kind = Some(AgentKind::Subagent);
        caller.subagent_id = Some("worker-child".into());
        caller.omp_session_id = Some("worker-child-native".into());
        denied.push(branch_subagent);
        for actor in denied {
            let bytes = fixture.state_bytes();
            let failure = if operation == 7 {
                fixture.service.mutate_reviewed(&actor, OrchestrationMutationRequest {
                    session_id: SESSION.into(), expected_revision: None, action: action.clone(),
                }, &fixture.run(&run_id))
            } else {
                fixture.apply(&actor, action.clone())
            }.unwrap_err();
            assert!(
                matches!(
                    failure.code.as_str(),
                    "actor_forbidden" | "caller_mismatch" | "session_mismatch" | "attempt_stale"
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
                    1 => run.stage = RunStage::Preparing,
                    2 => run.stage = RunStage::Closed,
                    3 => run.parent_run_id = Some(branch_id.clone()),
                    4 => run.root_id = other_root.clone(),
                    5 => run.bound_omp_session = None,
                    6 => run.bound_omp_session = Some(String::new()),
                    _ => unreachable!(),
                }
            });
            let bytes = fixture.state_bytes();
            let denied = if operation == 7 {
                fixture.service.mutate_reviewed(&supervisor, OrchestrationMutationRequest {
                    session_id: SESSION.into(), expected_revision: None, action: action.clone(),
                }, &fixture.run(&run_id))
            } else {
                fixture.apply(&supervisor, action.clone())
            };
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
fn supervisor_recovers_only_strict_descendants_with_review() {
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
    fixture.apply(&supervisor, OrchestrationAction::ReconcileRun {
        run_id: nested.clone(), recovery: None,
    }).unwrap();
    let reconciled = fixture.run(&nested);
    let annotation = reconciled.annotations.last().unwrap();
    assert!(matches!(&annotation.by, ActorRef::Run { run_id } if run_id == &root));
    assert!(annotation.text.contains("Supervisor"));
    assert!(annotation.text.contains(&reconciled.root_id));
    fixture.service.record_launch_review(&reconciled, DispatchStep::NeedsReview, Some(review_error())).unwrap();
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, OrchestrationAction::RetryLaunch {
        run_id: nested.clone(),
    }), "retry_preflight_required");
    assert_eq!(fixture.state_bytes(), bytes);
    let reviewed = fixture.run(&nested);
    fixture.seed_run(&nested, |run| run.bound_omp_process.as_mut().unwrap().start_ticks += 1);
    let bytes = fixture.state_bytes();
    assert_code(fixture.service.mutate_reviewed(&supervisor, OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None,
        action: OrchestrationAction::RetryLaunch { run_id: nested.clone() },
    }, &reviewed), "attempt_stale");
    assert_eq!(fixture.state_bytes(), bytes);
    let reviewed = fixture.run(&nested);
    let preserved = serde_json::to_value(&reviewed).unwrap();
    for target in [&root, &other_root, &outsider, &branch] {
        let actor = if target == &branch { &branch_actor } else { &supervisor };
        for operation in 0..2 {
            let bytes = fixture.state_bytes();
            let result = if operation == 0 {
                fixture.apply(actor, OrchestrationAction::ReconcileRun { run_id: target.clone(), recovery: None })
            } else {
                fixture.service.mutate_reviewed(actor, OrchestrationMutationRequest {
                    session_id: SESSION.into(), expected_revision: None,
                    action: OrchestrationAction::RetryLaunch { run_id: target.clone() },
                }, &fixture.run(target))
            };
            assert_code(result, "actor_forbidden");
            assert_eq!(fixture.state_bytes(), bytes);
        }
    }
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, OrchestrationAction::ReconcileRun {
        run_id: nested.clone(), recovery: Some(cockpit_protocol::projects::WorkspaceRecoveryAction::RetryEnvironment),
    }), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.service.mutate_reviewed(&supervisor, OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
        action: OrchestrationAction::RetryLaunch { run_id: nested.clone() },
    }, &reviewed).unwrap();
    let retried = fixture.run(&nested);
    assert_eq!(retried.dispatch.as_ref().unwrap().launch_attempt, reviewed.dispatch.as_ref().unwrap().launch_attempt + 1);
    assert_eq!(retried.stage, RunStage::Preparing);
    assert!(retried.location.is_none());
    let current = serde_json::to_value(&retried).unwrap();
    for field in ["task_id", "root_id", "setup", "grants", "work_plan", "last_report", "result"] {
        assert_eq!(current[field], preserved[field]);
    }
    assert!(retried.annotations.last().unwrap().text.contains("Supervisor"));
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
            OrchestrationAction::MessageSend {
                message_id: id(),
                to_run_id: nested,
                kind: MessageKind::Answer,
                text: "Too late".into(),
                in_reply_to: None,
            },
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
    let mut shell = actor.clone();
    caller_mut(&mut shell).actual_agent_kind = Some("shell".into());
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.apply(
            &shell,
            OrchestrationAction::RunAdopt {
                label: "Not OMP".into(),
            },
        ),
        "report_requires_main",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    let root = run_result(
        fixture
            .apply(
                &actor,
                OrchestrationAction::RunAdopt {
                    label: "Actual main explicitly adopted".into(),
                },
            )
            .unwrap(),
    )
    .0;
    caller_mut(&mut actor).env_run = Some((root.clone(), 1));
    assert_eq!(fixture.run(&root).kind, RunKind::Adopted);
    let task = fixture.create_task(&root, "Adopted root task");
    let run_id = fixture.propose(&root, &task, None);
    let plan = fixture.plan(&run_id);
    fixture
        .apply(
            &actor,
            OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: plan.plan_revision,
            },
        )
        .unwrap();
    let grant = fixture.run(&run_id).grants.remove(0);
    assert_eq!(grant.origin, GrantOrigin::Supervisor);
    assert_eq!(grant.supervisor_run_id.as_deref(), Some(root.as_str()));
    assert_eq!(grant.omp_session_id.as_deref(), Some("adopted-native"));
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
    fixture
        .apply(
            &worker,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
        .unwrap();
    let reviewed_result = fixture.run(&run_id).result.unwrap().message_id;
    let supervisor_native = fixture.run(&root).bound_omp_session;
    let intent_id = fixture.pending_intent(&root, &task, &run_id);
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let intent = state
            .task_intents
            .iter_mut()
            .find(|intent| intent.intent_id == intent_id)
            .unwrap();
        intent.origin = Some(GrantOrigin::Supervisor);
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
    fixture
        .apply(
            &parent_actor,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
        .unwrap();
    let parent_before = fixture.run(&parent);
    let parent_init = serde_json::to_value(parent_before.init_receipt.as_ref().unwrap()).unwrap();
    let parent_plan = serde_json::to_value(parent_before.work_plan.as_ref().unwrap()).unwrap();
    let parent_result = serde_json::to_value(parent_before.result.as_ref().unwrap()).unwrap();
    let assert_parent_receipts = || {
        let after = fixture.run(&parent);
        assert_eq!(after.stage, RunStage::Reported);
        assert_eq!(
            serde_json::to_value(after.init_receipt.as_ref().unwrap()).unwrap(),
            parent_init
        );
        assert_eq!(
            serde_json::to_value(after.work_plan.as_ref().unwrap()).unwrap(),
            parent_plan
        );
        assert_eq!(
            serde_json::to_value(after.result.as_ref().unwrap()).unwrap(),
            parent_result
        );
        assert_eq!(
            serde_json::to_value(after.last_report.as_ref().unwrap()).unwrap(),
            parent_result
        );
    };
    let child_before = fixture.run(&child);
    let child_init = serde_json::to_value(child_before.init_receipt.as_ref().unwrap()).unwrap();
    let child_plan = serde_json::to_value(child_before.work_plan.as_ref().unwrap()).unwrap();
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
                    to_run_id: None,
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
        assert_eq!(
            serde_json::to_value(child_after.init_receipt.as_ref().unwrap()).unwrap(),
            child_init
        );
        assert_eq!(
            serde_json::to_value(child_after.work_plan.as_ref().unwrap()).unwrap(),
            child_plan
        );
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
fn nested_setup_ready_result_and_question_pointers_wake_root_once_without_forwarding_subagent_evidence()
 {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let (_, branch, _) = fixture.working(&root);
    let task = fixture.create_task(&root, "Nested managed worker");
    let run_id = fixture.propose(&branch, &task, None);
    let prepare = fixture.plan(&run_id);
    let setup = fixture.run(&run_id).setup.unwrap();
    fixture
        .service
        .record_dispatch(
            &run_id,
            DispatchUpdate::SetupPlanned {
                setup,
                prepare_plan: prepare.clone(),
            },
        )
        .unwrap();
    let notice_id = format!("setup-ready-{run_id}-{}", prepare.plan_revision);
    let messages = fixture.state().messages;
    let notices: Vec<_> = messages
        .iter()
        .filter(|message| message.message_id == notice_id)
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].to_run_id, root);
    let pointer: serde_json::Value = serde_json::from_str(&notices[0].text).unwrap();
    assert_eq!(
        pointer,
        serde_json::json!({
            "event": "setup_ready", "run_id": run_id, "plan_revision": prepare.plan_revision,
        })
    );
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: prepare.plan_revision,
            },
        )
        .unwrap();
    let worker = fixture.launch_and_bind(&run_id);
    let ready = report(ReportKind::Ready, None, Some("Nested exact work plan"));
    let needs_input = report(ReportKind::NeedsInput, None, None);
    let result = report(ReportKind::Result, Some(ReportOutcome::Succeeded), None);
    for (event, action) in [
        ("ready", ready),
        ("needs_input", needs_input),
        ("result", result),
    ] {
        if event == "needs_input" {
            fixture
                .apply(
                    &supervisor,
                    OrchestrationAction::GrantExecute {
                        run_id: run_id.clone(),
                        plan_revision: fixture.run(&run_id).work_plan.unwrap().plan_revision,
                        note: None,
                    },
                )
                .unwrap();
        }
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
                "plan_revision": fixture.run(&run_id).work_plan.unwrap().plan_revision,
            })
        );
        assert!(!pointers[0].text.contains("Evidence from isolated worker"));
    }
    let original = serde_json::to_value(fixture.run(&run_id)).unwrap();
    let mut subagent = worker.clone();
    let caller = caller_mut(&mut subagent);
    caller.agent_kind = Some(AgentKind::Subagent);
    caller.subagent_id = Some("internal-evidence".into());
    caller.omp_session_id = Some("internal-evidence-native".into());
    let mut stale = worker;
    caller_mut(&mut stale).omp_session_id = Some("obsolete-native-main".into());
    for actor in [&subagent, &stale] {
        let action = report(ReportKind::NeedsInput, None, None);
        let OrchestrationAction::Report { message_id, .. } = &action else {
            unreachable!()
        };
        let pointer_id = format!("manage-{run_id}-{message_id}");
        fixture.apply(actor, action).unwrap();
        assert!(
            fixture
                .state()
                .messages
                .iter()
                .all(|message| message.message_id != pointer_id)
        );
        assert_eq!(
            serde_json::to_value(fixture.run(&run_id)).unwrap(),
            original
        );
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
    let child_before = serde_json::to_value(fixture.run(&child)).unwrap();
    let nested_before = serde_json::to_value(fixture.run(&nested)).unwrap();
    fixture.operator(OrchestrationAction::CancelRun {
        run_id: root.clone(),
    });
    assert_eq!(fixture.run(&root).stage, RunStage::Closed);
    assert_eq!(
        serde_json::to_value(fixture.run(&child)).unwrap(),
        child_before
    );
    assert_eq!(
        serde_json::to_value(fixture.run(&nested)).unwrap(),
        nested_before
    );
    assert!(!fixture.task(&root, &task.task_id).checked);
    let child_run = fixture.run(&child);
    let location = child_run.location.as_ref().unwrap();
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
    fixture
        .apply(
            &worker,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
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
fn supervisor_management_honors_explicit_machine_cas_without_requiring_new_consent() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "CAS reviewed setup");
    let run_id = fixture.propose(&root, &task, None);
    let plan = fixture.plan(&run_id);
    let reviewed_revision = fixture.state().revision;
    fixture.operator(OrchestrationAction::Annotate {
        run_id: root.clone(),
        text: "Concurrent unrelated machine history".into(),
    });
    assert_ne!(
        fixture.state().revision,
        reviewed_revision,
        "Concurrent machine history must invalidate an explicit machine CAS"
    );
    let bytes = fixture.state_bytes();
    assert_code(
        fixture.service.mutate(
            &supervisor,
            OrchestrationMutationRequest {
                session_id: SESSION.into(),
                expected_revision: Some(reviewed_revision),
                action: OrchestrationAction::GrantPrepare {
                    run_id: run_id.clone(),
                    plan_revision: plan.plan_revision.clone(),
                },
            },
        ),
        "orchestration_revision_conflict",
    );
    assert_eq!(fixture.state_bytes(), bytes);
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::GrantPrepare {
                run_id: run_id.clone(),
                plan_revision: plan.plan_revision,
            },
        )
        .unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Preparing);
    assert_eq!(
        fixture.run(&run_id).grants[0].origin,
        GrantOrigin::Supervisor
    );
}

#[test]
fn reviewed_retry_fences_post_read_cancel_move_binding_stage_and_incarnation() {
    for race in 0..6 {
        let fixture = Fixture::new();
        let (run_id, actor) = launched_case(&fixture, RunStage::Working);
        let queued = fixture.queue_review(&run_id);
        commit_review(
            &fixture,
            &queued,
            DispatchStep::NeedsReview,
            Some(review_error()),
        );
        let reviewed = fixture.service.run_for_review(SESSION, &run_id).unwrap();
        match race {
            0 => {
                fixture.operator(OrchestrationAction::CancelRun {
                    run_id: run_id.clone(),
                });
            }
            1 => {
                let mut moved = actor.clone();
                let caller = caller_mut(&mut moved);
                caller.workspace_id = "post-read-space".into();
                caller.tab_id = "post-read-tab".into();
                caller.pane_id = "post-read-pane".into();
                fixture
                    .apply(
                        &moved,
                        OrchestrationAction::Annotate {
                            run_id: run_id.clone(),
                            text: "Post-read terminal move".into(),
                        },
                    )
                    .unwrap();
            }
            2 => fixture.seed_run(&run_id, |run| {
                run.bound_omp_session = Some("post-read-main".into())
            }),
            3 => {
                fixture
                    .apply(
                        &actor,
                        report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
                    )
                    .unwrap();
            }
            4 => fixture.seed_run(&run_id, |run| {
                run.dispatch.as_mut().unwrap().launch_attempt += 1
            }),
            5 => fixture.seed_run(&run_id, |run| run.task_id = Some(id())),
            _ => unreachable!(),
        }
        let bytes = fixture.state_bytes();
        assert_code(
            fixture.service.mutate_reviewed(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationMutationRequest {
                    session_id: SESSION.into(),
                    expected_revision: Some(fixture.state().revision),
                    action: OrchestrationAction::RetryLaunch { run_id },
                },
                &reviewed,
            ),
            "attempt_stale",
        );
        assert_eq!(fixture.state_bytes(), bytes);
    }
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
        fixture
            .apply(
                &actor,
                report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
            )
            .unwrap();
        let intent_id = fixture.pending_intent(&root, &task, &run_id);
        fixture.seed_run(&run_id, |run| match race {
            0 => {
                run.stage = RunStage::Closed;
                run.close_reason = Some(CloseReason::Cancelled);
            }
            1 => {
                run.stage = RunStage::Preparing;
                run.bound_omp_session = None;
                run.dispatch.as_mut().unwrap().launch_attempt += 1;
            }
            2 => run.result.as_mut().unwrap().outcome = Some(ReportOutcome::Failed),
            3 => run.result = None,
            4 => run.task_id = Some(id()),
            5 => run.root_id = id(),
            6 => run.stage = RunStage::Working,
            _ => unreachable!(),
        });
        let before = serde_json::to_value(fixture.run(&run_id)).unwrap();
        let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
        reopened.recover_intents().unwrap();
        assert_eq!(serde_json::to_value(fixture.run(&run_id)).unwrap(), before);
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
    let worker = fixture.propose(&root, &task, None);
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
    assert_eq!(serde_json::to_value(reopened_state).unwrap(), serde_json::to_value(fixture.state()).unwrap());
}

#[test]
fn launch_review_failure_classifies_effect_and_operator_cases() {
    for case in 0..7 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Classified recovery");
        let worker = fixture.propose(&root, &task, None);
        fixture.prepare(&worker);
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
            if case == 1 { run.bound_omp_process = None; }
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
        let (effect, next) = match case {
            0 => ("unknown", serde_json::json!(["show", "reconcile", "retry_launch"])),
            1 => ("unknown", serde_json::json!(["show", "reconcile", "operator"])),
            2 => ("unknown", serde_json::json!(["show", "operator"])),
            3 => ("unknown", serde_json::json!(["show", "reconcile_accept_existing_worktree", "operator"])),
            4 => ("none", serde_json::json!(["show", "retry_launch"])),
            _ => unreachable!(),
        };
        assert_eq!(body["effect"], effect);
        assert_eq!(body["next"], next);
        assert_eq!(body["operator_reason"].is_string(), matches!(case, 1..=3));
        assert!(body["error"]["message"].as_str().unwrap().len() <= 1024);
    }
}

fn observed_root_pane(fixture: &Fixture, root: &str) -> herdr::RuntimePane {
    let location = fixture.run(root).location.unwrap();
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
        let worker = fixture.propose(&root, &task, None);
        fixture.prepare(&worker);
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
            assert_eq!(body["next"], serde_json::json!(["show", "retry_launch"]));
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
        fixture.apply(&actor, OrchestrationAction::RunBindSession {
            omp_session_id: caller.omp_session_id.clone().unwrap(),
        }).unwrap();
        let reviewed = fixture.run(&worker);
        fixture.service.record_launch_verified(&reviewed).unwrap();
        let notices = dispatch_events(&fixture, &root, "dispatch_recovered");
        assert_eq!(notices.len(), usize::from(exhausted));
        let bytes = fixture.state_bytes();
        fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
        assert_eq!(fixture.state_bytes(), bytes);
        assert_eq!(dispatch_events(&fixture, &root, "dispatch_recovered").len(), usize::from(exhausted));
        if exhausted {
            let queued = fixture.queue_review(&worker);
            fixture.service.record_launch_review(&queued, DispatchStep::Launched, None).unwrap();
            assert_eq!(dispatch_events(&fixture, &root, "dispatch_recovered").len(), 1);
        }
    }
}

#[test]
fn supervisor_recovery_preserves_machine_cas_and_automatic_launch_ownership() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let (_, worker, _) = fixture.working(&root);
    let queued = fixture.queue_review(&worker);
    fixture.service.record_launch_review(&queued, DispatchStep::NeedsReview, Some(review_error())).unwrap();
    let reviewed = fixture.run(&worker);
    let revision = fixture.state().revision;
    fixture.operator(OrchestrationAction::Annotate { run_id: root.clone(), text: "Unrelated newer history".into() });
    for action in [
        OrchestrationAction::ReconcileRun { run_id: worker.clone(), recovery: None },
        OrchestrationAction::RetryLaunch { run_id: worker.clone() },
    ] {
        let request = OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: Some(revision), action };
        let bytes = fixture.state_bytes();
        let result = if matches!(&request.action, OrchestrationAction::RetryLaunch { .. }) {
            fixture.service.mutate_reviewed(&supervisor, request, &reviewed)
        } else {
            fixture.service.mutate(&supervisor, request)
        };
        assert_code(result, "orchestration_revision_conflict");
        assert_eq!(fixture.state_bytes(), bytes);
    }
    fixture.seed_run(&worker, |run| run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending);
    let reviewed = fixture.run(&worker);
    let bytes = fixture.state_bytes();
    assert_code(fixture.service.mutate_reviewed(&supervisor, OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None,
        action: OrchestrationAction::RetryLaunch { run_id: worker.clone() },
    }, &reviewed), "invalid_stage");
    assert_eq!(fixture.state_bytes(), bytes);
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
    assert!(recovery.annotations.last().unwrap().text.contains("Supervisor"));
}

#[test]
fn old_unknown_launch_with_fresh_bound_omp_proof_stays_quiet_before_reconcile() {
    for evidence in 0..5 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Recover an old launch outcome");
        let worker = fixture.propose(&root, &task, None);
        fixture.prepare(&worker);
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
        let location = fixture.run(&worker).location.unwrap();
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
    let run_id = fixture.propose(root, task, None);
    fixture.prepare(&run_id);
    let actor = fixture.launch_and_bind(&run_id);
    fixture.ready(&actor);
    fixture.operator(OrchestrationAction::GrantExecute {
        run_id: run_id.clone(), plan_revision: fixture.run(&run_id).work_plan.unwrap().plan_revision, note: None,
    });
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
    for missing in 0..5 {
        fixture.seed_run(&worker, |run| {
            *run = original.clone();
            match missing {
                0 => run.init_receipt = None,
                1 => run.grants.retain(|grant| grant.scope != GrantScope::Execute),
                2 => run.work_plan.as_mut().unwrap().plan_revision = "different-work-plan".into(),
                3 => run.bound_omp_process = None,
                _ => run.init_receipt.as_mut().unwrap().plan = Some("Unreviewed initialization".into()),
            }
        });
        let bytes = std::fs::read(&path).unwrap();
        assert_code(fixture.apply(&actor, content_actions(&root, &own).remove(0)), "actor_forbidden");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        for manager in [&root_actor, &operator] {
            assert_code(fixture.apply(manager, content_actions(&root, &own).remove(0)), "actor_forbidden");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    for stage in [RunStage::Preparing, RunStage::Ready, RunStage::Reported, RunStage::Closed] {
        fixture.seed_run(&worker, |run| { *run = original.clone(); run.stage = stage; });
        let bytes = std::fs::read(&path).unwrap();
        assert_code(fixture.apply(&actor, content_actions(&root, &own).remove(0)),
            if stage == RunStage::Closed { "attempt_stale" } else { "actor_forbidden" });
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
fn child_content_authority_requires_self_authenticated_binding_not_parent_telemetry() {
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
        label: "Parent telemetry cannot mint a native binding".into(), status: SubagentStatus::Running, summary: None,
    };
    fixture.apply(&parent, update.clone()).unwrap();
    assert!(fixture.state().subagents[0].bound_omp_session.is_none());
    let mut child = parent.clone();
    let caller = caller_mut(&mut child);
    caller.agent_kind = Some(AgentKind::Subagent);
    caller.subagent_id = Some("actual-child".into());
    caller.omp_session_id = Some("actual-child-session".into());
    let check = |task: &Task| OrchestrationAction::TaskStepSetChecked {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
        step_id: step_id.clone(), checked: true, scope: TaskStepScope::Leaf,
    };
    assert_code(fixture.apply(&child, check(&task)), "actor_forbidden");
    fixture.apply(&child, update.clone()).unwrap();
    assert_eq!(fixture.state().subagents[0].bound_omp_session.as_deref(), Some("actual-child-session"));
    fixture.apply(&parent, update.clone()).unwrap();
    assert_eq!(fixture.state().subagents[0].bound_omp_session.as_deref(), Some("actual-child-session"));
    let checked = task_result(fixture.apply(&child, check(&task)).unwrap());
    assert_eq!(checked.step_progress.as_ref().map(|progress| (progress.done, progress.total)), Some((1, 1)));
    assert!(!checked.checked, "Step completion is not task acceptance");
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
    let sibling = fixture.create_task(&root, "Not the child's task");
    assert_code(fixture.apply(&child, content_actions(&root, &sibling).remove(0)), "actor_forbidden");
    let mut forged = child.clone();
    caller_mut(&mut forged).omp_session_id = Some("unbound-child-session".into());
    assert_code(fixture.apply(&forged, check(&checked)), "actor_forbidden");
    caller_mut(&mut forged).omp_session_id = Some("actual-child-session".into());
    caller_mut(&mut forged).process.as_mut().unwrap().start_ticks += 1;
    assert_code(fixture.apply(&forged, check(&checked)), "actor_forbidden");
    let mut done = update;
    if let OrchestrationAction::SubagentUpdate { status, .. } = &mut done {
        *status = SubagentStatus::Done;
    }
    fixture.apply(&child, done).unwrap();
    assert_code(fixture.apply(&child, check(&checked)), "actor_forbidden");
}

#[test]
fn dependencies_gate_prepare_before_supersession_and_current_attempt_stays_with_live_worker() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Prerequisite");
    canonical_check(&fixture, &root, &source.task_id, true);
    let task = related_task(&fixture, &root, &[source.task_id.clone()], None);
    let (old, _) = execute_task(&fixture, &root, &task);
    canonical_check(&fixture, &root, &source.task_id, false);
    let replacement = fixture.propose(&root, &task, Some(&old));
    let plan = fixture.plan(&replacement);
    assert_eq!(projection::current_task_run(&fixture.state(), &root, &task.task_id).unwrap().run_id, old);
    let state = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::GrantPrepare {
        run_id: replacement.clone(), plan_revision: plan.plan_revision.clone(),
    }), "task_blocked");
    assert_eq!(fixture.state_bytes(), state);
    assert_eq!(fixture.run(&old).stage, RunStage::Working);
    assert!(fixture.run(&replacement).grants.is_empty());
    canonical_check(&fixture, &root, &source.task_id, true);
    fixture.operator(OrchestrationAction::GrantPrepare { run_id: replacement.clone(), plan_revision: plan.plan_revision });
    assert_eq!(fixture.run(&old).close_reason, Some(CloseReason::Superseded));
    assert_eq!(projection::current_task_run(&fixture.state(), &root, &task.task_id).unwrap().run_id, replacement);
}

#[test]
fn dependency_changes_outside_target_bytes_gate_execute_accept_sendback_and_dispatch() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Prerequisite");
    canonical_check(&fixture, &root, &source.task_id, true);
    let task = related_task(&fixture, &root, &[source.task_id.clone()], None);
    let run_id = fixture.propose(&root, &task, None);
    fixture.prepare(&run_id);
    let actor = fixture.launch_and_bind(&run_id);
    fixture.ready(&actor);
    canonical_check(&fixture, &root, &source.task_id, false);
    assert_eq!(fixture.task(&root, &task.task_id).task_revision, task.task_revision);
    assert!(!fixture.service.dispatch_task_eligible(&fixture.run(&run_id)).unwrap());
    let state = fixture.state_bytes();
    let execute = OrchestrationAction::GrantExecute {
        run_id: run_id.clone(), plan_revision: fixture.run(&run_id).work_plan.unwrap().plan_revision, note: None,
    };
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), execute.clone()), "task_blocked");
    assert_eq!(fixture.state_bytes(), state);
    // No global task freeze: healthy independent work can still be granted.
    let independent = fixture.create_task(&root, "Independent task");
    let (independent_run, _) = execute_task(&fixture, &root, &independent);
    assert!(fixture.service.dispatch_task_eligible(&fixture.run(&independent_run)).unwrap());
    canonical_check(&fixture, &root, &source.task_id, true);
    fixture.operator(execute);
    canonical_check(&fixture, &root, &source.task_id, false);
    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let before = std::fs::read(&path).unwrap();
    for action in content_actions(&root, &task) {
        assert_code(fixture.apply(&actor, action), "task_blocked");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    canonical_check(&fixture, &root, &source.task_id, true);
    fixture.apply(&actor, report(ReportKind::Result, Some(ReportOutcome::Succeeded), None)).unwrap();
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
            fixture.apply(&actor, report(ReportKind::Result, Some(ReportOutcome::Succeeded), None)).unwrap();
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
    fixture.propose(&root, &task, None); // Proposed is live, even before Prepare.
    // External source can be corrupted. Removing an unrelated edge is still
    // monotone, and is allowed even when the remaining source edge stays cyclic.
    {
        let locked = fixture.service.store.lock().unwrap();
        let document = locked.tasks(&root).unwrap();
        let source = document.task(&source.task_id).unwrap();
        document.set_dependencies(&source.task_id, &source.task_revision, &document.doc_revision,
            &[task.task_id.clone()]).unwrap();
    }
    let worker_task = fixture.create_task(&root, "Independent worker");
    let (_, worker) = execute_task(&fixture, &root, &worker_task);
    let action = |edges: Vec<String>| OrchestrationAction::TaskDependenciesSet {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
        expected_doc_revision: task_document_revision(&fixture, &root), depends_on: edges,
    };
    assert_code(fixture.apply(&worker, action(vec![source.task_id.clone()])), "actor_forbidden");
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        action(vec![source.task_id.clone(), extra.task_id.clone()])), "task_relationships_live");
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        action(vec![source.task_id.clone(), worker_task.task_id.clone()])), "task_relationships_live");
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
        TaskDependencyState::Invalid);
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
    fixture.apply(&actor, report(ReportKind::Result, Some(ReportOutcome::Succeeded), None)).unwrap();
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
fn proposing_waiting_work_is_non_destructive_but_checked_or_invalid_tasks_are_rejected_locally() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Waiting prerequisite");
    let waiting = related_task(&fixture, &root, &[source.task_id.clone()], None);
    let run_id = fixture.propose(&root, &waiting, None);
    let plan = fixture.plan(&run_id);
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::GrantPrepare {
        run_id: run_id.clone(), plan_revision: plan.plan_revision,
    }), "task_blocked");
    assert_eq!(fixture.run(&run_id).stage, RunStage::AwaitingPrepare);
    assert!(!fixture.service.dispatch_task_eligible(&fixture.run(&run_id)).unwrap());
    let invalid = fixture.create_task(&root, "Corrupt component");
    let invalid = {
        let locked = fixture.service.store.lock().unwrap();
        let document = locked.tasks(&root).unwrap();
        document.set_dependencies(&invalid.task_id, &invalid.task_revision, &document.doc_revision, &[id()]).unwrap()
    };
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), proposal(&root, &invalid, None)),
        "task_dependencies_invalid");
    let independent = fixture.create_task(&root, "Healthy independent component");
    let healthy = fixture.propose(&root, &independent, None);
    fixture.prepare(&healthy);
    assert!(fixture.service.dispatch_task_eligible(&fixture.run(&healthy)).unwrap());
    canonical_check(&fixture, &root, &source.task_id, true);
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        proposal(&root, &fixture.task(&root, &source.task_id), None)), "task_checked");
}
