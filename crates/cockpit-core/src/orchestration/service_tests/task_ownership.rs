use super::*;

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
fn blocked_dependencies_never_replace_the_current_live_worker() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let source = fixture.create_task(&root, "Prerequisite");
    canonical_check(&fixture, &root, &source.task_id, true);
    let task = related_task(&fixture, &root, &[source.task_id.clone()], None);
    let (old, _) = execute_task(&fixture, &root, &task);
    canonical_check(&fixture, &root, &source.task_id, false);
    let supervisor = fixture.root_actor(&root);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor,
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
    let supervisor = fixture.root_actor(&root);
    assert!(!fixture.service.dispatch_task_eligible(&fixture.run(&run_id)).unwrap());
    let invalid = fixture.create_task(&root, "Corrupt component");
    let invalid = {
        let locked = fixture.service.store.lock().unwrap();
        let document = locked.tasks(&root).unwrap();
        document.set_dependencies(&invalid.task_id, &invalid.task_revision, &document.doc_revision, &[id()]).unwrap()
    };
    assert_code(fixture.apply(&supervisor, fixture.start_action(&root, &invalid)),
        "task_dependencies_invalid");
    let independent = fixture.create_task(&root, "Healthy independent component");
    let healthy = fixture.start(&root, &independent);
    fixture.setup(&healthy);
    assert!(fixture.service.dispatch_task_eligible(&fixture.run(&healthy)).unwrap());
    canonical_check(&fixture, &root, &source.task_id, true);
    assert_code(fixture.apply(&supervisor,
        fixture.start_action(&root, &fixture.task(&root, &source.task_id))), "task_checked");
}


#[test]
fn worker_start_requires_current_canonical_task_and_document_revisions_without_writes() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Start revision fence");
    let mut action = fixture.start_action(&root, &task);
    fixture.create_task(&root, "Unrelated document edit");
    let supervisor = fixture.root_actor(&root);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, action.clone()),
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
    assert_code(fixture.apply(&supervisor, action),
        "task_revision_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    assert!(fixture.state().runs.iter().all(|run| run.kind != RunKind::Worker));
}

fn ownership(fixture: &Fixture, root: &str, task: &Task) -> TaskOwnership {
    fixture.state().task_ownership.into_iter().find(|record|
        record.root_id == root && record.task_id == task.task_id).unwrap()
}

fn unassign(root: &str, task: &Task, generation: u64) -> OrchestrationAction {
    OrchestrationAction::TaskUnassign {
        operation_id: id(), root_id: root.into(), task_id: task.task_id.clone(),
        expected_assignment_generation: generation,
    }
}

fn pending_worker(fixture: &Fixture, root: &str, task: &Task) -> (String, Actor) {
    let run = fixture.start(root, task);
    let actor = fixture.neutral_actor(&run);
    (run, actor)
}

#[test]
fn concurrent_conversation_claimants_publish_exactly_one_atomic_owner() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Atomic claim");
    let (run, first) = pending_worker(&fixture, &root, &task);
    let mut second = first.clone();
    caller_mut(&mut second).conversation.session_id = "different-opaque-chat".into();
    assert_eq!(ownership(&fixture, &root, &task).generation, 0);
    assert!(ownership(&fixture, &root, &task).owner.is_none());
    let operation = id();
    let barrier = std::sync::Barrier::new(3);
    let (a, b) = std::thread::scope(|scope| {
        let attempt = |actor: &Actor| {
            barrier.wait();
            fixture.apply(actor, OrchestrationAction::RunRegister { operation_id: operation.clone() })
        };
        let first = &first;
        let second = &second;
        let a = scope.spawn(move || attempt(first));
        let b = scope.spawn(move || attempt(second));
        barrier.wait();
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_ne!(a.is_ok(), b.is_ok(), "Exactly one competing conversation may claim");
    let winner = if a.is_ok() { first } else { second };
    let Actor::Agent(caller) = winner else { unreachable!() };
    let record = ownership(&fixture, &root, &task);
    assert_eq!(record.generation, 1);
    let owner = record.owner.unwrap();
    assert_eq!(owner.run_id, run);
    assert_eq!(owner.conversation, caller.conversation);
    assert_eq!(owner.claim_operation_id, operation);
    assert_eq!(fixture.run(&run).attachment.unwrap().conversation, Some(owner.conversation));
    assert_eq!(fixture.state().messages.iter().filter(|message|
        message.to_run_id == run && message.kind == MessageKind::WorkBrief).count(), 1);
}

#[test]
fn registration_replay_and_implicit_same_chat_context_are_inert_but_other_chat_conflicts() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Explicit then implicit registration");
    let (run, actor) = pending_worker(&fixture, &root, &task);
    let action = OrchestrationAction::RunRegister { operation_id: id() };
    let first = fixture.apply(&actor, action.clone()).unwrap();
    assert!(matches!(first.result, OrchestrationActionResult::RunRegistered {
        assignment_generation: Some(1), duplicate: false, ..
    }));
    let bytes = fixture.state_bytes();
    let revision = fixture.state().revision;
    let receiver = fixture.service.subscribe();
    for replay in [
        action.clone(),
        OrchestrationAction::RunRegister { operation_id: caller_auth(&actor).credential_id.clone() },
        OrchestrationAction::RunRegister { operation_id: id() },
    ] {
        let result = fixture.apply(&actor, replay).unwrap();
        assert!(matches!(result.result, OrchestrationActionResult::RunRegistered { duplicate: true, .. }));
        assert_eq!(fixture.state_bytes(), bytes);
        assert_eq!(fixture.state().revision, revision);
        assert!(!receiver.has_changed().unwrap());
    }
    for field in 0..2 {
        let mut other = actor.clone();
        let caller = caller_mut(&mut other);
        if field == 0 { caller.conversation.session_id = "other-chat".into(); }
        else { caller.conversation.harness = "other-harness".into(); }
        assert!(fixture.apply(&other, action.clone()).is_err());
        assert!(fixture.apply(&other, report(ReportKind::Progress, None, None)).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    assert_eq!(ownership(&fixture, &root, &task).owner.unwrap().run_id, run);
}

#[test]
fn worker_start_is_root_only_and_preclaim_capability_cannot_write() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Pending authority");
    let root_actor = fixture.root_actor(&root);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        fixture.start_action(&root, &task)), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), bytes);
    let (run, actor) = pending_worker(&fixture, &root, &task);
    let bytes = fixture.state_bytes();
    for action in content_actions(&root, &task).into_iter().chain([
        report(ReportKind::Progress, None, None),
        fixture.successful_result(&actor),
        OrchestrationAction::MessageSend {
            message_id: id(), to_run_id: root.clone(), kind: MessageKind::Observation,
            text: "Not admitted before registration".into(), in_reply_to: None,
            original_task_revision: None, original_claim_id: None,
        },
    ]) {
        assert!(fixture.apply(&actor, action).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    assert_eq!(fixture.run(&run).assignment_generation, Some(1));
    assert_eq!(caller_auth(&actor).assignment_generation, Some(1));
    assert_eq!(ownership(&fixture, &root, &task).generation, 0);
    fixture.apply(&actor, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
    assert_eq!(fixture.run(&run).stage, RunStage::Working);
    assert_code(fixture.apply(&actor, fixture.start_action(&root, &task)), "actor_forbidden");
    assert!(fixture.apply(&root_actor, unassign(&root, &task, 1)).is_ok());
}

#[test]
fn unassign_is_active_same_root_only_and_exact_generation_fenced() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run, worker) = fixture.working(&root);
    let supervisor = fixture.root_actor(&root);
    let other_root = fixture.root();
    let outsider = fixture.root_actor(&other_root);
    let mut other_chat = supervisor.clone();
    caller_mut(&mut other_chat).conversation.session_id = "different-supervisor-chat".into();
    let action = unassign(&root, &task, 1);
    let bytes = fixture.state_bytes();
    for actor in [&worker, &outsider, &other_chat, &Actor::Operator(OperatorOrigin::Browser)] {
        assert!(fixture.apply(actor, action.clone()).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    for stage in [RunStage::Starting, RunStage::Reported, RunStage::Closed] {
        fixture.seed_run(&root, |run| run.stage = stage);
        let bytes = fixture.state_bytes();
        assert!(fixture.apply(&supervisor, action.clone()).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    fixture.seed_run(&root, |run| run.stage = RunStage::Active);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, unassign(&root, &task, 0)), "assignment_generation_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    let first = fixture.apply(&supervisor, action.clone()).unwrap();
    assert!(matches!(&first.result, OrchestrationActionResult::TaskUnassigned {
        assignment_generation: 2, previous_run_id: Some(previous), duplicate: false, ..
    } if previous == &run));
    let bytes = fixture.state_bytes();
    let replay = fixture.apply(&supervisor, action).unwrap();
    assert!(matches!(replay.result, OrchestrationActionResult::TaskUnassigned { duplicate: true, .. }));
    assert_eq!(fixture.state_bytes(), bytes);
    assert_code(fixture.apply(&supervisor, unassign(&root, &task, 1)), "assignment_generation_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn pending_unassign_revokes_late_registration_launch_and_all_old_capabilities() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Revoke pending attempt");
    let supervisor = fixture.root_actor(&root);
    let (run, actor) = pending_worker(&fixture, &root, &task);
    let pending = fixture.run(&run);
    fixture.apply(&supervisor, unassign(&root, &task, 0)).unwrap();
    let record = ownership(&fixture, &root, &task);
    assert_eq!(record.generation, 1);
    assert!(record.owner.is_none());
    assert_eq!(fixture.run(&run).stage, RunStage::Closed);
    assert_eq!(fixture.run(&run).close_reason, Some(CloseReason::Superseded));
    assert!(fixture.state().credentials.iter().filter(|record| record.run_id == run)
        .all(|record| record.state == credentials::CredentialState::Revoked));
    let bytes = fixture.state_bytes();
    assert!(fixture.apply(&actor, OrchestrationAction::RunRegister {
        operation_id: caller_auth(&actor).credential_id.clone(),
    }).is_err());
    assert!(fixture.apply(&actor, report(ReportKind::Progress, None, None)).is_err());
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.service.record_launch_verified(&pending).unwrap();
    assert_eq!(fixture.state_bytes(), bytes, "Late launch acknowledgement must not resurrect revoked pending work");
    let (replacement, next) = pending_worker(&fixture, &root, &task);
    assert_ne!(replacement, run);
    assert_eq!(caller_auth(&next).assignment_generation, Some(2));
    fixture.apply(&next, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
    assert_eq!(ownership(&fixture, &root, &task).generation, 2);
}

#[test]
fn same_conversation_aba_never_revives_old_generation_or_replayed_worker_writes() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "ABA fencing");
    let supervisor = fixture.root_actor(&root);
    let (old, worker) = pending_worker(&fixture, &root, &task);
    let registration = OrchestrationAction::RunRegister { operation_id: id() };
    fixture.apply(&worker, registration.clone()).unwrap();
    let progress = report(ReportKind::Progress, None, None);
    fixture.apply(&worker, progress.clone()).unwrap();
    fixture.apply(&supervisor, unassign(&root, &task, 1)).unwrap();
    let (next, mut replacement) = pending_worker(&fixture, &root, &task);
    caller_mut(&mut replacement).conversation = match &worker {
        Actor::Agent(caller) => caller.conversation.clone(), _ => unreachable!(),
    };
    fixture.apply(&replacement, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
    let record = ownership(&fixture, &root, &task);
    assert_eq!(record.generation, 3);
    assert_eq!(record.owner.unwrap().run_id, next);
    assert_ne!(old, next);
    let bytes = fixture.state_bytes();
    for action in content_actions(&root, &task).into_iter().chain([
        registration, progress, fixture.successful_result(&worker),
    ]) {
        assert!(fixture.apply(&worker, action).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    for field in 0..2 {
        let mut stale = replacement.clone();
        if field == 0 { caller_mut(&mut stale).assignment_generation = Some(1); }
        else { caller_mut(&mut stale).auth.assignment_generation = Some(1); }
        assert!(fixture.apply(&stale, report(ReportKind::Progress, None, None)).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
}

#[test]
fn revoked_generation_rejects_scope_messages_resource_mutations_and_message_replay() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run, worker) = fixture.working(&root);
    let supervisor = fixture.root_actor(&root);
    let attachment = fixture.run(&run).attachment.unwrap();
    let resource_request = OrchestrationAction::ResourceRequest { request: ResourceRequest {
        operation_id: id(), resource_key: fixture.root.join("build-output").to_string_lossy().into_owned(),
        source_identity: ResourceSourceIdentity {
            checkout_key: fixture.run(&run).checkout.unwrap().key,
            source_commit: "a".repeat(40), source_tree: "b".repeat(40),
            input_configuration: "service-tests".into(), toolchain_fingerprint: "test-toolchain".into(),
        },
        holder: cockpit_protocol::orchestration::AttachmentReference {
            run_id: run.clone(), attachment_id: attachment.attachment_id.clone(),
            attachment_epoch: attachment.epoch,
        },
    }};
    let message = OrchestrationAction::MessageSend {
        message_id: id(), to_run_id: root.clone(), kind: MessageKind::Observation,
        text: "Captured worker evidence".into(), in_reply_to: None,
        original_task_revision: None, original_claim_id: None,
    };
    fixture.apply(&worker, message.clone()).unwrap();
    let OrchestrationAction::ResourceRequest { request } = &resource_request else { unreachable!() };
    let holder = request.holder.clone();
    let request_id = request.operation_id.clone();
    fixture.seed_state(|state| {
        state.resource_sequence = 1;
        state.resource_requests.push(ResourceRequestRecord {
            request_id: request_id.clone(), sequence: 1, resource_key: request.resource_key.clone(),
            source_identity: request.source_identity.clone(), holder: holder.clone(),
            state: ResourceRequestState::Held, build_processes: Vec::new(),
            machine_id: "test-machine".into(), blocked_reason: None, updated_at: now(),
        });
    });
    let requests = serde_json::to_value(fixture.state().resource_requests).unwrap();
    fixture.apply(&supervisor, unassign(&root, &task, 1)).unwrap();
    let bytes = fixture.state_bytes();
    for action in [
        resource_request, message,
        OrchestrationAction::ResourceRelease { release: ResourceRelease {
            request_id: request_id.clone(), expected_holder: holder.clone(),
        }},
        OrchestrationAction::ResourceBuildRegister { registration: ResourceBuildRegistration {
            request_id, expected_holder: holder, process: live_process(),
        }},
        OrchestrationAction::TaskScopeUpdate {
            operation_id: id(), root_id: root.clone(), task_id: task.task_id.clone(), run_id: run.clone(),
            expected_task_revision: task.task_revision.clone(),
            expected_doc_revision: task_document_revision(&fixture, &root),
            title: None, description: Some("Revoked scope".into()),
        },
        OrchestrationAction::ResourceRegister { request: ResourceRegister {
            operation_id: id(), run_id: run.clone(),
            resource: OwnedResourceIdentity::Space { identity: OwnedSpaceIdentity {
                creation_operation_id: id(), endpoint_identity: "endpoint-one".into(),
                session_id: SESSION.into(), workspace_id: "test-space".into(),
                checkout_path: fixture.root.to_string_lossy().into_owned(),
            }},
        }},
        report(ReportKind::NeedsInput, None, None),
    ] {
        assert!(fixture.apply(&worker, action).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    assert_eq!(serde_json::to_value(fixture.state().resource_requests).unwrap(), requests);
}

#[test]
fn unassign_preserves_source_result_claim_location_and_history_without_process_exit() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run, worker) = fixture.working(&root);
    let supervisor = fixture.root_actor(&root);
    fixture.seed_state(|state| state.owned_resources.push(OwnedResourceRecord {
        resource_id: id(), root_id: root.clone(), run_id: run.clone(),
        kind: OwnedResourceKind::Space, ownership: ResourceOwnership::Owned,
        creation_operation_id: Some(id()),
        identity: OwnedResourceIdentity::Space { identity: OwnedSpaceIdentity {
            creation_operation_id: id(), endpoint_identity: "endpoint-one".into(),
            session_id: SESSION.into(), workspace_id: "test-space".into(),
            checkout_path: fixture.root.to_string_lossy().into_owned(),
        }},
        authority: CleanupAuthority::OwnedWhenSafe, consumers: Vec::new(), integration_result_id: None,
        cleanup: CleanupItem { operation_id: None, state: CleanupState::Waiting, reason: None,
            effects: Vec::new(), updated_at: now() },
    }));
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    let before = fixture.run(&run);
    let claims = serde_json::to_value(fixture.state().checkout_claims).unwrap();
    let resources = serde_json::to_value(fixture.state().owned_resources).unwrap();
    let result = before.result.as_ref().unwrap().message_id.clone();
    fixture.apply(&supervisor, unassign(&root, &task, 1)).unwrap();
    let after = fixture.run(&run);
    assert_eq!(after.stage, RunStage::Closed);
    assert_eq!(after.close_reason, Some(CloseReason::Superseded));
    assert_eq!(after.source_output, before.source_output);
    assert_eq!(serde_json::to_value(&after.result).unwrap(), serde_json::to_value(&before.result).unwrap());
    assert_eq!(serde_json::to_value(&after.attachment.as_ref().unwrap().location).unwrap(),
        serde_json::to_value(&before.attachment.as_ref().unwrap().location).unwrap());
    assert_eq!(serde_json::to_value(&after.attachment).unwrap(), serde_json::to_value(&before.attachment).unwrap());
    assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
    assert_eq!(serde_json::to_value(fixture.state().checkout_claims).unwrap(), claims);
    assert_eq!(serde_json::to_value(fixture.state().owned_resources).unwrap(), resources);
    assert!(fixture.state().messages.iter().any(|message| message.message_id == result));
    assert!(!fixture.task(&root, &task.task_id).checked);
    let bytes = fixture.state_bytes();
    assert!(fixture.apply(&worker, fixture.successful_result(&worker)).is_err());
    assert_eq!(fixture.state_bytes(), bytes);
    let (replacement, actor) = pending_worker(&fixture, &root, &task);
    fixture.apply(&actor, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
    assert_eq!(ownership(&fixture, &root, &task).owner.unwrap().run_id, replacement);
    let history = fixture.run(&run);
    assert_eq!(history.source_output, before.source_output);
    assert_eq!(history.result.unwrap().message_id, result);
    assert_eq!(history.checkout_claim_id, before.checkout_claim_id);
    assert_eq!(projection::current_task_run(&fixture.state(), &root, &task.task_id).unwrap().run_id, replacement);
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    let state = reopened.store.lock().unwrap().read().unwrap();
    let persisted = state.task_ownership.iter().find(|record| record.task_id == task.task_id).unwrap();
    assert_eq!(persisted.generation, 3);
    assert_eq!(persisted.owner.as_ref().unwrap().run_id, replacement);
    assert!(state.runs.iter().any(|record| record.run_id == run && record.result.is_some()));
}

#[test]
fn accepted_task_cannot_be_unassigned_or_registered_again() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run, worker) = fixture.working(&root);
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    fixture.apply(&supervisor, OrchestrationAction::Accept {
        run_id: run, expected_task_revision: task.task_revision.clone(),
    }).unwrap();
    let bytes = fixture.state_bytes();
    assert!(fixture.apply(&supervisor, unassign(&root, &task, 1)).is_err());
    assert!(fixture.apply(&worker, OrchestrationAction::RunRegister { operation_id: id() }).is_err());
    assert_eq!(fixture.state_bytes(), bytes);
    assert!(fixture.task(&root, &task.task_id).checked);
}

#[test]
fn root_registration_pins_opaque_chat_and_conflicting_replay_never_changes_it() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut actor = fixture.neutral_actor(&root);
    caller_mut(&mut actor).conversation.session_id = "not/a/uuid:conversation".into();
    let action = OrchestrationAction::RunRegister { operation_id: id() };
    fixture.apply(&actor, action.clone()).unwrap();
    let bytes = fixture.state_bytes();
    fixture.apply(&actor, action.clone()).unwrap();
    assert_eq!(fixture.state_bytes(), bytes);
    let mut other = actor.clone();
    caller_mut(&mut other).conversation.session_id = "different-root-chat".into();
    assert!(fixture.apply(&other, action).is_err());
    assert_eq!(fixture.state_bytes(), bytes);
    assert_eq!(fixture.run(&root).assignment_generation, None);
    assert!(fixture.state().task_ownership.is_empty());
}

#[test]
fn concurrent_root_authorizations_reserve_only_one_pending_attempt() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Single pending authorization");
    let supervisor = fixture.root_actor(&root);
    let first = fixture.start_action(&root, &task);
    let second = fixture.start_action(&root, &task);
    let barrier = std::sync::Barrier::new(3);
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| { barrier.wait(); fixture.apply(&supervisor, first) });
        let b = scope.spawn(|| { barrier.wait(); fixture.apply(&supervisor, second) });
        barrier.wait();
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(fixture.state().runs.iter().filter(|run|
        run.task_id.as_deref() == Some(task.task_id.as_str()) && run.stage != RunStage::Closed).count(), 1);
    let record = ownership(&fixture, &root, &task);
    assert_eq!(record.generation, 0);
    assert!(record.owner.is_none());
}

#[test]
fn unassign_generation_overflow_fails_without_releasing_owner_or_claims() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, worker, _) = fixture.working(&root);
    let supervisor = fixture.root_actor(&root);
    fixture.seed_state(|state| {
        state.task_ownership.iter_mut()
            .find(|record| record.task_id == task.task_id).unwrap().generation = MAX_SAFE_COUNTER;
        state.runs.iter_mut().find(|run| run.run_id == worker).unwrap()
            .assignment_generation = Some(MAX_SAFE_COUNTER);
        for credential in &mut state.credentials {
            if credential.run_id == worker {
                credential.assignment_generation = Some(MAX_SAFE_COUNTER);
            }
        }
    });
    let before = fixture.state();
    assert!(ownership(&fixture, &root, &task).owner.is_some());
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, unassign(&root, &task, MAX_SAFE_COUNTER)), "invalid_counter");
    assert_eq!(fixture.state_bytes(), bytes);
    let after = fixture.state();
    assert_eq!(serde_json::to_value(after.task_ownership).unwrap(),
        serde_json::to_value(before.task_ownership).unwrap());
    assert_eq!(serde_json::to_value(after.checkout_claims).unwrap(),
        serde_json::to_value(before.checkout_claims).unwrap());
    assert_eq!(serde_json::to_value(after.credentials).unwrap(),
        serde_json::to_value(before.credentials).unwrap());
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
}
