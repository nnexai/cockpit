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
        let configuration: ProjectConfiguration = serde_json::from_value(serde_json::json!({
            "version": 1,
            "repository_roots": [],
            "worktree_root": root.join("worktrees"),
            "companion_root": root.join("unused-companions"),
            "state_root": root.join("state"),
            "cache_root": root.join("cache"),
            "library_root": root.join("library"),
            "notes_root": root.join("notes"),
            "branch_template": "test/{task}",
            "checkout_template": "{task}",
            "providers": [],
            "origins": {},
            "limits": {
                "catalog_depth": 1, "catalog_entries": 1,
                "git_timeout_ms": 1000, "git_output_bytes": 1024,
                "operation_timeout_ms": 1000,
                "context_preview_bytes": 1024, "context_preview_lines": 10,
                "context_directory_entries": 10, "context_tree_depth": 1,
                "library_folder_files": 10, "library_folder_bytes": 1024,
                "library_file_bytes": 1024, "library_space_pages": 10,
                "library_attachment_bytes": 1024,
                "library_item_attachment_bytes": 1024, "library_max_items": 10
            }
        }))
        .unwrap();
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
        task_result(self.operator(OrchestrationAction::TaskCreate {
            root_id: root_id.into(),
            title: title.into(),
            body: "Canonical task body".into(),
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
        state.task_intents.push(TaskIntent {
            intent_id: intent_id.clone(),
            root_id: root_id.into(),
            task_id: task.task_id.clone(),
            run_id: run_id.into(),
            expected_task_revision: task.task_revision.clone(),
            state: IntentState::Pending,
            origin: None,
            supervisor_run_id: None,
            omp_session_id: None,
            result_message_id: None,
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
                    body: "Own body".into(),
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
                    body: None,
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
            body: String::new(),
        },
        OrchestrationAction::TaskUpdate {
            root_id: other_root.clone(),
            task_id: other_task.task_id.clone(),
            expected_task_revision: other_task.task_revision.clone(),
            title: Some("Intrusion".into()),
            body: None,
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
        body: None,
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
        let edited = task_result(fixture.operator(OrchestrationAction::TaskUpdate {
            root_id: root.clone(),
            task_id: task.task_id.clone(),
            expected_task_revision: task.task_revision.clone(),
            title: Some("Changed while acceptance was interrupted".into()),
            body: None,
        }));
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
        let recovered_revision = state.revision;
        reopened.recover_intents().unwrap();
        assert_eq!(
            reopened.store.lock().unwrap().read().unwrap().revision,
            recovered_revision
        );
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
        reopened.recover_intents().unwrap();
        assert!(fixture.task(&root, &task.task_id).checked);
        assert_eq!(
            fixture.run(&run_id).close_reason,
            Some(CloseReason::Accepted)
        );
        assert!(fixture.state().task_intents.is_empty());
        let revision = fixture.state().revision;
        reopened.recover_intents().unwrap();
        assert_eq!(fixture.state().revision, revision);
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
    assert_eq!(
        actual, expected,
        "Review must not change other runs or inboxes"
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
            assert_eq!(
                serde_json::to_value(fixture.state().messages).unwrap(),
                messages
            );
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
        let dispatch = retried.dispatch.as_ref().unwrap();
        assert_eq!(dispatch.step, DispatchStep::SetupPending);
        assert_eq!(
            dispatch.launch_attempt,
            original.dispatch.as_ref().unwrap().launch_attempt + 1
        );
        assert!(!dispatch.agent_started);
        assert!(dispatch.launch_tag.is_none());
        assert!(dispatch.endpoint_identity.is_none());
        let mut expected = serde_json::to_value(&original).unwrap();
        let actual = serde_json::to_value(&retried).unwrap();
        for field in [
            "stage",
            "location",
            "bound_omp_session",
            "dispatch",
            "updated_at",
        ] {
            expected[field] = actual[field].clone();
        }
        assert_eq!(actual, expected);
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
    let answer_id = id();
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::MessageSend {
                message_id: answer_id.clone(),
                to_run_id: run_id.clone(),
                kind: MessageKind::Answer,
                text: "Use the established task scope; no additional decision is missing".into(),
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
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    fixture
        .apply(
            &worker,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
        .unwrap();
    assert!(!fixture.task(&root, &task.task_id).checked);
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
                    body: Some("Canonical task body with reviewed evidence requirements".into()),
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
    for operation in 0..6 {
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
            let failure = fixture.apply(&actor, action.clone()).unwrap_err();
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
            assert!(fixture.apply(&supervisor, action.clone()).is_err());
            assert_eq!(
                fixture.state_bytes(),
                bytes,
                "Malformed root must not gain management authority"
            );
        }
    }
}

#[test]
fn management_targets_are_strict_open_worker_descendants_and_recovery_stays_operator_only() {
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
    for action in [
        OrchestrationAction::ReconcileRun {
            run_id: nested.clone(),
            recovery: None,
        },
        OrchestrationAction::RetryLaunch {
            run_id: nested.clone(),
        },
        OrchestrationAction::CancelRun {
            run_id: branch.clone(),
        },
    ] {
        let bytes = fixture.state_bytes();
        let actor = if matches!(&action, OrchestrationAction::CancelRun { .. }) {
            &branch_actor
        } else {
            &supervisor
        };
        assert_code(fixture.apply(actor, action), "actor_forbidden");
        assert_eq!(fixture.state_bytes(), bytes);
    }
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
}

#[test]
fn legacy_schema_one_grants_and_acceptance_intents_keep_truthful_default_provenance() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, actor) = fixture.working(&root);
    fixture
        .apply(
            &actor,
            report(ReportKind::Result, Some(ReportOutcome::Succeeded), None),
        )
        .unwrap();
    fixture.pending_intent(&root, &task, &run_id);
    let mut legacy = serde_json::to_value(fixture.state()).unwrap();
    legacy.as_object_mut().unwrap().remove("assignment_intents");
    let run = legacy["runs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|run| run["run_id"] == run_id)
        .unwrap();
    for (index, origin) in ["browser", "native"].into_iter().enumerate() {
        let grant = run["grants"][index].as_object_mut().unwrap();
        grant.insert("origin".into(), serde_json::json!(origin));
        grant.remove("supervisor_run_id");
        grant.remove("omp_session_id");
    }
    for intent in legacy["task_intents"].as_array_mut().unwrap() {
        let intent = intent.as_object_mut().unwrap();
        for key in [
            "origin",
            "supervisor_run_id",
            "omp_session_id",
            "result_message_id",
        ] {
            intent.remove(key);
        }
    }
    std::fs::write(
        fixture.service.base().join("state.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    let locked = reopened.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    assert_eq!(state.schema, 1);
    assert!(state.assignment_intents.is_empty());
    let current = &state.runs[run_index(&state, &run_id).unwrap()];
    assert_eq!(current.grants[0].origin, GrantOrigin::Browser);
    assert_eq!(current.grants[1].origin, GrantOrigin::Native);
    for grant in &current.grants {
        assert!(grant.supervisor_run_id.is_none());
        assert!(grant.omp_session_id.is_none());
    }
    let intent = &state.task_intents[0];
    assert!(intent.origin.is_none());
    assert!(intent.supervisor_run_id.is_none());
    assert!(intent.omp_session_id.is_none());
    assert!(intent.result_message_id.is_none());
    locked.save(&mut state).unwrap();
    let mut saved = serde_json::to_value(locked.read().unwrap()).unwrap();
    assert_eq!(saved["schema"], legacy["schema"]);
    saved["revision"] = legacy["revision"].clone();
    saved.as_object_mut().unwrap().remove("assignment_intents");
    let run = saved["runs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|run| run["run_id"] == run_id)
        .unwrap();
    for grant in run["grants"].as_array_mut().unwrap() {
        let grant = grant.as_object_mut().unwrap();
        assert_eq!(
            grant.remove("supervisor_run_id"),
            Some(serde_json::Value::Null)
        );
        assert_eq!(
            grant.remove("omp_session_id"),
            Some(serde_json::Value::Null)
        );
    }
    for intent in saved["task_intents"].as_array_mut().unwrap() {
        for key in [
            "origin",
            "supervisor_run_id",
            "omp_session_id",
            "result_message_id",
        ] {
            assert_eq!(
                intent.as_object_mut().unwrap().remove(key),
                Some(serde_json::Value::Null)
            );
        }
    }
    assert_eq!(
        saved, legacy,
        "Resaving schema 1 must not rewrite history or invent provenance"
    );
    drop(locked);
    assert!(!fixture.task(&root, &task.task_id).checked);
    assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
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
        body: "Canonical body is never copied to inbox".into(),
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
                body: "Submitted body".into(),
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
            fixture.service.mutate_operator_reviewed(
                OperatorOrigin::Browser,
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
                body: "Do not overwrite authoritative task".into(),
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
