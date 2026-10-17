use super::*;

struct AuthorityFixture {
    fixture: Fixture,
    root: String,
    other_root: String,
    worker: String,
    task: Task,
    root_actor: Actor,
    owner: Actor,
    sibling: Actor,
    foreign_root: Actor,
    foreign_worker: Actor,
    parent_step: String,
    child_step: String,
}

impl AuthorityFixture {
    fn new() -> Self {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (mut task, worker, owner) = fixture.working(&root);
        let parent_step = id();
        let child_step = id();
        for (step_id, parent_step_id) in [
            (parent_step.clone(), None),
            (child_step.clone(), Some(parent_step.clone())),
        ] {
            task = task_result(fixture.apply(&owner, OrchestrationAction::TaskStepAdd {
                root_id: root.clone(), task_id: task.task_id.clone(),
                expected_task_revision: task.task_revision, step_id, parent_step_id,
                before_step_id: None, title: "Existing checklist step".into(),
            }).unwrap());
        }
        let (_, _, sibling) = fixture.working(&root);
        let other_root = fixture.root();
        let (_, _, foreign_worker) = fixture.working(&other_root);
        let root_actor = fixture.root_actor(&root);
        let foreign_root = fixture.root_actor(&other_root);
        Self {
            fixture, root, other_root, worker, task, root_actor, owner, sibling,
            foreign_root, foreign_worker, parent_step, child_step,
        }
    }

    fn current_task(&self) -> Task {
        self.fixture.task(&self.root, &self.task.task_id)
    }

    fn markdown(&self) -> Vec<Vec<u8>> {
        [&self.root, &self.other_root].into_iter().map(|root| {
            std::fs::read(self.fixture.service.base().join("tasks").join(format!("{root}.md"))).unwrap()
        }).collect()
    }

    fn denied(&self, actor_name: &str, actor: &Actor, action: OrchestrationAction, code: &str) {
        let state = self.fixture.state_bytes();
        let markdown = self.markdown();
        let label = format!("{actor_name}: {action:?}");
        let failure = self.fixture.apply(actor, action).unwrap_err();
        assert_eq!(failure.code, code, "{label}: {failure:?}");
        assert_eq!(self.fixture.state_bytes(), state, "Denied state write: {label}");
        assert_eq!(self.markdown(), markdown, "Denied Markdown write: {label}");
    }

    fn content_actions(&self) -> Vec<OrchestrationAction> {
        let task = self.current_task();
        let root_id = self.root.clone();
        let task_id = task.task_id;
        let expected_task_revision = task.task_revision;
        vec![
            OrchestrationAction::TaskUpdate {
                root_id: root_id.clone(), task_id: task_id.clone(), expected_task_revision: expected_task_revision.clone(),
                title: Some("Refined Working task".into()), description: Some("Durable revised scope".into()),
            },
            OrchestrationAction::TaskStepAdd {
                root_id: root_id.clone(), task_id: task_id.clone(), expected_task_revision: expected_task_revision.clone(),
                step_id: id(), parent_step_id: None, before_step_id: None, title: "Added step".into(),
            },
            OrchestrationAction::TaskStepRename {
                root_id: root_id.clone(), task_id: task_id.clone(), expected_task_revision: expected_task_revision.clone(),
                step_id: self.child_step.clone(), title: "Renamed child".into(),
            },
            OrchestrationAction::TaskStepSetChecked {
                root_id: root_id.clone(), task_id: task_id.clone(), expected_task_revision: expected_task_revision.clone(),
                step_id: self.child_step.clone(), checked: true, scope: TaskStepScope::Leaf,
            },
            OrchestrationAction::TaskStepMove {
                root_id: root_id.clone(), task_id: task_id.clone(), expected_task_revision: expected_task_revision.clone(),
                step_id: self.child_step.clone(), parent_step_id: None, before_step_id: Some(self.parent_step.clone()),
            },
            OrchestrationAction::TaskStepRemove {
                root_id, task_id, expected_task_revision, step_id: self.parent_step.clone(),
            },
        ]
    }

    fn scope_action(&self) -> OrchestrationAction {
        let task = self.current_task();
        OrchestrationAction::TaskScopeUpdate {
            operation_id: id(), root_id: self.root.clone(), task_id: task.task_id,
            run_id: self.worker.clone(), expected_task_revision: task.task_revision,
            expected_doc_revision: task_document_revision(&self.fixture, &self.root),
            title: Some("Refined Working task".into()), description: Some("Durable revised scope".into()),
        }
    }

    fn denied_actors(&self) -> Vec<(&'static str, Actor, &'static str)> {
        let mut stale_generation = self.owner.clone();
        caller_mut(&mut stale_generation).assignment_generation = Some(
            caller_auth(&self.owner).assignment_generation.unwrap() - 1,
        );
        let mut stale_auth_generation = self.owner.clone();
        caller_mut(&mut stale_auth_generation).auth.assignment_generation = Some(
            caller_auth(&self.owner).assignment_generation.unwrap() - 1,
        );
        let mut stale_epoch = self.owner.clone();
        caller_mut(&mut stale_epoch).auth.attachment_epoch += 1;
        let mut invalid_credential = self.owner.clone();
        caller_mut(&mut invalid_credential).auth.credential_id = id();
        vec![
            ("sibling", self.sibling.clone(), "actor_forbidden"),
            ("foreign root", self.foreign_root.clone(), "actor_forbidden"),
            ("foreign worker", self.foreign_worker.clone(), "actor_forbidden"),
            ("stale declared generation", stale_generation, "job_unauthorized"),
            ("stale credential generation", stale_auth_generation, "job_unauthorized"),
            ("stale attachment epoch", stale_epoch, "job_unauthorized"),
            ("unknown credential", invalid_credential, "job_unauthorized"),
        ]
    }
}

#[test]
fn working_content_and_checklist_authority_matrix_preserves_denied_bytes() {
    let matrix = AuthorityFixture::new();
    for (name, actor, code) in matrix.denied_actors() {
        for action in matrix.content_actions().into_iter().chain([matrix.scope_action()]) {
            matrix.denied(name, &actor, action, code);
        }
    }
    // Operator follow-ups retain the explicit, durable TaskScopeUpdate contract.
    for action in matrix.content_actions() {
        matrix.denied("operator Working content", &Actor::Operator(OperatorOrigin::Browser), action, "actor_forbidden");
    }
}

#[test]
fn root_and_own_worker_edit_working_content_and_every_checklist_action() {
    for root_author in [true, false] {
        let matrix = AuthorityFixture::new();
        let actor = if root_author { &matrix.root_actor } else { &matrix.owner };
        let initial = matrix.fixture.run(&matrix.worker);
        for index in 0..6 {
            let before = matrix.current_task();
            let state = matrix.fixture.state();
            let response = matrix.fixture.apply(actor, matrix.content_actions().remove(index)).unwrap();
            // TaskUpdate keeps its public Task receipt, including when the root
            // routes a Working edit through the durable scope-update journal.
            let task = task_result(response);
            assert_ne!(task.task_revision, before.task_revision);
            assert_eq!(task.task_revision, matrix.current_task().task_revision);
            assert!(!task.checked);
            match index {
                0 => {
                    assert_eq!(task.title, "Refined Working task");
                    assert_eq!(task.description, "Durable revised scope");
                    let marker = "<!-- cockpit-checklist: begin -->";
                    assert_eq!(&task.body[task.body.find(marker).unwrap()..],
                        &before.body[before.body.find(marker).unwrap()..]);
                    let after = matrix.fixture.state();
                    if root_author {
                        assert!(after.scope_intents.is_empty());
                        assert_eq!(after.operations.len(), state.operations.len() + 1);
                        let operation = after.operations.last().unwrap();
                        assert_eq!(operation.kind, store::DurableOperationKind::TaskScopeUpdate);
                        assert_eq!(operation.actor_run_id.as_deref(), Some(matrix.root.as_str()));
                        let Some(OrchestrationActionResult::TaskScopeUpdated { update }) = &operation.result else {
                            panic!("Root TaskUpdate must retain the durable scope receipt");
                        };
                        assert_eq!(update.state, TaskScopeUpdateState::Applied);
                        assert_eq!(update.task_revision.as_deref(), Some(task.task_revision.as_str()));
                        assert_eq!(after.messages.len(), state.messages.len() + 1);
                        let instruction = after.messages.last().unwrap();
                        assert_eq!(instruction.kind, MessageKind::Instruction);
                        assert_eq!(instruction.to_run_id, matrix.worker);
                        assert!(matches!(&instruction.from, ActorRef::Run { run_id } if run_id == &matrix.root));
                        assert_eq!(update.instruction_message_id.as_deref(), Some(instruction.message_id.as_str()));
                        let pointer: serde_json::Value = serde_json::from_str(&instruction.text).unwrap();
                        assert_eq!(pointer["event"], "task_scope_updated");
                        assert_eq!(pointer["operation_id"], operation.operation_id);
                        assert_eq!(pointer["root_id"], matrix.root);
                        assert_eq!(pointer["task_id"], task.task_id);
                        assert_eq!(pointer["task_revision"], task.task_revision);
                        assert_eq!(pointer["doc_revision"], task_document_revision(&matrix.fixture, &matrix.root));
                    } else {
                        assert_eq!(after.messages.len(), state.messages.len());
                        assert_eq!(after.operations.len(), state.operations.len());
                    }
                }
                1 => assert_eq!(task.steps.len(), 3),
                2 => assert_eq!(task.steps[1].title, "Renamed child"),
                3 => assert!(task.steps[1].checked),
                4 => {
                    assert_eq!(task.steps[0].step_id.as_deref(), Some(matrix.child_step.as_str()));
                    assert!(task.steps[0].checked);
                }
                5 => assert_eq!(task.steps.len(), 2),
                _ => unreachable!(),
            }
            let run = matrix.fixture.run(&matrix.worker);
            assert_eq!(run.stage, RunStage::Working);
            assert_eq!(run.assignment_generation, initial.assignment_generation);
            assert_eq!(run.attachment.as_ref().unwrap().epoch, initial.attachment.as_ref().unwrap().epoch);
            assert_eq!(run.checkout_claim_id, initial.checkout_claim_id);
        }
    }
}

#[test]
fn explicit_scope_update_allows_root_owner_operator_and_notifies_only_other_authors() {
    for author in ["root", "owner", "operator"] {
        let matrix = AuthorityFixture::new();
        let operator = Actor::Operator(OperatorOrigin::Browser);
        let actor = match author { "root" => &matrix.root_actor, "owner" => &matrix.owner, _ => &operator };
        let action = matrix.scope_action();
        let before = matrix.fixture.state();
        let response = matrix.fixture.apply(actor, action.clone()).unwrap();
        let OrchestrationActionResult::TaskScopeUpdated { update } = response.result else { panic!("Scope receipt") };
        assert_eq!(update.state, TaskScopeUpdateState::Applied);
        assert_eq!(update.instruction_message_id.is_some(), author != "owner");
        assert_eq!(matrix.current_task().description, "Durable revised scope");
        let after = matrix.fixture.state();
        assert!(after.scope_intents.is_empty());
        assert_eq!(after.messages.len(), before.messages.len() + usize::from(author != "owner"));
        let bytes = matrix.fixture.state_bytes();
        let markdown = matrix.markdown();
        let replay = matrix.fixture.apply(actor, action).unwrap();
        let OrchestrationActionResult::TaskScopeUpdated { update } = replay.result else { panic!("Scope replay receipt") };
        assert!(update.duplicate);
        assert_eq!(matrix.fixture.state_bytes(), bytes);
        assert_eq!(matrix.markdown(), markdown);
    }
}

#[test]
fn accepted_tasks_are_immutable_to_root_operator_and_retired_worker() {
    let matrix = AuthorityFixture::new();
    matrix.fixture.apply(&matrix.owner, matrix.fixture.successful_result(&matrix.owner)).unwrap();
    matrix.fixture.apply(&matrix.root_actor, OrchestrationAction::Accept {
        run_id: matrix.worker.clone(), expected_task_revision: matrix.current_task().task_revision,
    }).unwrap();
    assert!(matrix.current_task().checked);
    assert_eq!(matrix.fixture.run(&matrix.worker).close_reason, Some(CloseReason::Accepted));
    for (name, actor, code) in [
        ("root", &matrix.root_actor, "task_checked"),
        ("operator", &Actor::Operator(OperatorOrigin::Browser), "task_checked"),
        ("retired owner", &matrix.owner, "job_unauthorized"),
    ] {
        for action in matrix.content_actions().into_iter().chain([matrix.scope_action()]) {
            matrix.denied(name, actor, action, code);
        }
    }
}

#[test]
fn root_authoring_execution_and_management_deny_workers_and_foreign_roots() {
    let matrix = AuthorityFixture::new();
    let task = matrix.current_task();
    let actions = vec![
        OrchestrationAction::TaskCreate {
            root_id: matrix.root.clone(), task_id: id(), title: "Root-only creation".into(), description: "New task".into(),
            depends_on: Vec::new(), follow_up_of: None, expected_doc_revision: None, source_revision: None,
        },
        OrchestrationAction::TaskDependenciesSet {
            root_id: matrix.root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
            expected_doc_revision: task_document_revision(&matrix.fixture, &matrix.root), depends_on: Vec::new(),
        },
        OrchestrationAction::TasksAssignIds {
            root_id: matrix.root.clone(), expected_doc_revision: task_document_revision(&matrix.fixture, &matrix.root),
        },
        OrchestrationAction::TaskUnassign {
            operation_id: id(), root_id: matrix.root.clone(), task_id: task.task_id.clone(),
            expected_assignment_generation: caller_auth(&matrix.owner).assignment_generation.unwrap(),
        },
        matrix.fixture.start_action(&matrix.root, &task),
        OrchestrationAction::TaskSchedule {
            operation_id: id(), root_id: matrix.root.clone(), task_id: task.task_id.clone(), target: Some(target()),
            brief: "Schedule canonical task".into(), cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe, label: None,
            expected_task_revision: Some(task.task_revision.clone()),
            expected_doc_revision: Some(task_document_revision(&matrix.fixture, &matrix.root)),
        },
        OrchestrationAction::TaskScheduleCancel {
            operation_id: id(), root_id: matrix.root.clone(), task_id: task.task_id.clone(),
        },
        OrchestrationAction::WorkerReplace {
            operation_id: id(), root_id: matrix.root.clone(), task_id: task.task_id.clone(), expected_run_id: matrix.worker.clone(),
            expected_assignment_generation: caller_auth(&matrix.owner).assignment_generation.unwrap(), reason: "Replacement review".into(),
        },
        OrchestrationAction::Accept { run_id: matrix.worker.clone(), expected_task_revision: task.task_revision },
        OrchestrationAction::SendBack { run_id: matrix.worker.clone(), text: "Review correction".into() },
        OrchestrationAction::CancelRun { run_id: matrix.worker.clone() },
        OrchestrationAction::ReconcileRun { run_id: matrix.worker.clone(), recovery: None },
    ];
    for (name, actor, code) in matrix.denied_actors().into_iter().chain([
        ("own worker", matrix.owner.clone(), "actor_forbidden"),
    ]) {
        for action in &actions {
            matrix.denied(name, &actor, action.clone(), code);
        }
    }
}

#[test]
fn operator_only_intents_never_gain_root_or_worker_authority() {
    let matrix = AuthorityFixture::new();
    let attachment = matrix.fixture.run(&matrix.worker).attachment.unwrap();
    let actions = vec![
        OrchestrationAction::TaskAssign {
            root_id: matrix.root.clone(), task_id: id(), title: "Operator assignment".into(), description: "Operator provenance".into(),
        },
        OrchestrationAction::TaskAssignmentResolve {
            root_id: matrix.root.clone(), task_id: matrix.task.task_id.clone(), expected_task_revision: Some(matrix.current_task().task_revision), assign: false,
        },
        OrchestrationAction::IntentResolve { intent_id: id(), apply: false },
        OrchestrationAction::SupervisorStart {
            operation_id: id(), mode: SupervisorStartMode::Another, target: Some(target()), label: None,
            selected_root_id: None, expected_project_key: Some(matrix.fixture.project().key),
        },
        OrchestrationAction::RunAdopt { label: "Operator adoption".into(), location: attachment.location.clone().unwrap() },
        OrchestrationAction::RunAttach {
            operation_id: id(), attachment_id: attachment.attachment_id, expected_attachment_epoch: attachment.epoch,
            conversation: attachment.conversation.clone().unwrap(),
        },
        OrchestrationAction::ReconcileRun {
            run_id: matrix.worker.clone(), recovery: Some(cockpit_protocol::projects::WorkspaceRecoveryAction::RetryEnvironment),
        },
    ];
    for (name, actor) in [("root", &matrix.root_actor), ("owner", &matrix.owner), ("sibling", &matrix.sibling), ("foreign root", &matrix.foreign_root)] {
        for action in &actions {
            matrix.denied(name, actor, action.clone(), "actor_forbidden");
        }
    }
}

#[test]
fn durable_generation_and_credential_revocation_cannot_retain_content_authority() {
    for fence in ["generation", "credential", "epoch"] {
        let matrix = AuthorityFixture::new();
        matrix.fixture.seed_state(|state| match fence {
            "generation" => {
                let ownership = state.task_ownership.iter_mut().find(|record|
                    record.root_id == matrix.root && record.task_id == matrix.task.task_id).unwrap();
                ownership.generation += 1;
                let run = state.runs.iter_mut().find(|run| run.run_id == matrix.worker).unwrap();
                run.assignment_generation = Some(ownership.generation);
            }
            "credential" => {
                let credential = state.credentials.iter_mut().find(|credential|
                    credential.credential_id == caller_auth(&matrix.owner).credential_id).unwrap();
                credential.state = credentials::CredentialState::Revoked;
            }
            _ => {
                let run = state.runs.iter_mut().find(|run| run.run_id == matrix.worker).unwrap();
                run.attachment.as_mut().unwrap().epoch += 1;
            }
        });
        for action in matrix.content_actions().into_iter().chain([matrix.scope_action()]) {
            matrix.denied(fence, &matrix.owner, action, "job_unauthorized");
        }
    }
}

#[test]
fn cleanup_scope_and_resource_registration_preserve_owner_and_evidence_boundaries() {
    let matrix = AuthorityFixture::new();
    let cleanup = OrchestrationAction::CleanupFinish { request: CleanupFinish {
        operation_id: id(), root_id: matrix.root.clone(), run_id: Some(matrix.worker.clone()),
    }};
    let register = OrchestrationAction::ResourceRegister { request: ResourceRegister {
        operation_id: id(), run_id: matrix.worker.clone(),
        resource: OwnedResourceIdentity::Space { identity: OwnedSpaceIdentity {
            creation_operation_id: id(), endpoint_identity: "endpoint-one".into(),
            session_id: SESSION.into(), workspace_id: "test-space".into(),
            checkout_path: matrix.fixture.run(&matrix.worker).checkout.unwrap().path,
        }},
    }};
    for (name, actor, code) in matrix.denied_actors() {
        for action in [&cleanup, &register] {
            matrix.denied(name, &actor, action.clone(), code);
        }
    }
    let operator = Actor::Operator(OperatorOrigin::Browser);
    for (name, actor) in [("root", &matrix.root_actor), ("owner", &matrix.owner), ("operator", &operator)] {
        // Authority is not a substitute for an actual creation-adapter receipt.
        matrix.denied(name, actor, register.clone(), "resource_creation_unverified");
        let mut action = cleanup.clone();
        let OrchestrationAction::CleanupFinish { request } = &mut action else { unreachable!() };
        request.operation_id = id();
        let markdown = matrix.markdown();
        let response = matrix.fixture.apply(actor, action.clone()).unwrap();
        assert!(matches!(response.result, OrchestrationActionResult::CleanupFinished { .. }));
        assert_eq!(matrix.markdown(), markdown);
        let bytes = matrix.fixture.state_bytes();
        matrix.fixture.apply(actor, action).unwrap();
        assert_eq!(matrix.fixture.state_bytes(), bytes);
    }
    matrix.denied("worker root-wide cleanup", &matrix.owner,
        OrchestrationAction::CleanupFinish { request: CleanupFinish {
            operation_id: id(), root_id: matrix.root.clone(), run_id: None,
        }}, "actor_forbidden");
}

#[test]
fn accepted_integration_record_is_root_or_operator_only_with_immutable_markdown() {
    let matrix = AuthorityFixture::new();
    matrix.fixture.apply(&matrix.owner, matrix.fixture.successful_result(&matrix.owner)).unwrap();
    let task_revision = matrix.current_task().task_revision;
    matrix.fixture.apply(&matrix.root_actor, OrchestrationAction::Accept {
        run_id: matrix.worker.clone(), expected_task_revision: task_revision.clone(),
    }).unwrap();
    let action = OrchestrationAction::IntegrationRecord { record: IntegrationRecord {
        operation_id: id(), run_id: matrix.worker.clone(),
        expected_result_message_id: matrix.fixture.run(&matrix.worker).result.unwrap().message_id,
        expected_task_revision: task_revision, integration: matrix.fixture.run(&matrix.worker).integration,
    }};
    for (name, actor, code) in matrix.denied_actors() {
        matrix.denied(name, &actor, action.clone(), code);
    }
    matrix.denied("retired owner", &matrix.owner, action.clone(), "job_unauthorized");
    let operator = Actor::Operator(OperatorOrigin::Browser);
    for actor in [&matrix.root_actor, &operator] {
        let mut action = action.clone();
        let OrchestrationAction::IntegrationRecord { record } = &mut action else { unreachable!() };
        record.operation_id = id();
        let markdown = matrix.markdown();
        let result = matrix.fixture.apply(actor, action.clone()).unwrap();
        let OrchestrationActionResult::IntegrationRecorded { receipt } = result.result else { panic!("Integration receipt") };
        assert!(matches!(receipt.integration, Integration::NotApplicable { .. }));
        assert_eq!(matrix.markdown(), markdown);
        let bytes = matrix.fixture.state_bytes();
        let replay = matrix.fixture.apply(actor, action).unwrap();
        let OrchestrationActionResult::IntegrationRecorded { receipt } = replay.result else { panic!("Integration replay") };
        assert!(receipt.duplicate);
        assert_eq!(matrix.fixture.state_bytes(), bytes);
        assert_eq!(matrix.markdown(), markdown);
    }
}
