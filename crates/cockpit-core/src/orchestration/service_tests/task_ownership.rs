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
fn content_actions_preserve_worker_ownership_and_operator_working_scope_gate() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, worker, actor) = fixture.working(&root);
    let sibling = fixture.create_task(&root, "Sibling canonical task");
    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let root_actor = fixture.launch_and_bind(&root);
    let operator = Actor::Operator(OperatorOrigin::Browser);
    for action in content_actions(&root, &task) {
        let bytes = std::fs::read(&path).unwrap();
        assert_code(fixture.apply(&operator, action), "actor_forbidden");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
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
    caller_mut(&mut second).conversation.as_mut().unwrap().session_id = "different-opaque-chat".into();
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
    assert_eq!(fixture.run(&run).attachment.unwrap().conversation, owner.conversation);
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
        if field == 0 { caller.conversation.as_mut().unwrap().session_id = "other-chat".into(); }
        else { caller.conversation.as_mut().unwrap().harness = "other-harness".into(); }
        assert!(fixture.apply(&other, action.clone()).is_err());
        assert!(fixture.apply(&other, report(ReportKind::Progress, None, None)).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    assert_eq!(ownership(&fixture, &root, &task).owner.unwrap().run_id, run);
}

#[test]
fn worker_start_is_root_or_operator_only_and_preclaim_capability_cannot_write() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Pending authority");
    let root_actor = fixture.root_actor(&root);
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
    caller_mut(&mut other_chat).conversation.as_mut().unwrap().session_id = "different-supervisor-chat".into();
    let action = unassign(&root, &task, 1);
    let bytes = fixture.state_bytes();
    for actor in [&worker, &outsider, &other_chat] {
        assert!(fixture.apply(actor, action.clone()).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    let operator = Actor::Operator(OperatorOrigin::Native);
    assert!(fixture.apply(&operator, unassign(&other_root, &task, 1)).is_err());
    assert_code(fixture.service.mutate_resolved(&operator, OrchestrationMutationRequest {
        session_id: "foreign-session".into(), expected_revision: None, action: action.clone(),
    }, &ResolvedMutationInput::default()), "session_mismatch");
    assert_eq!(fixture.state_bytes(), bytes);
    for stage in [RunStage::Starting, RunStage::Reported, RunStage::Closed] {
        fixture.seed_run(&root, |run| run.stage = stage);
        let bytes = fixture.state_bytes();
        assert!(fixture.apply(&supervisor, action.clone()).is_err());
        assert!(fixture.apply(&operator, action.clone()).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    fixture.seed_run(&root, |run| run.stage = RunStage::Active);
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, unassign(&root, &task, 0)), "assignment_generation_conflict");
    assert_code(fixture.apply(&operator, unassign(&root, &task, 0)), "assignment_generation_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    let first = fixture.apply(&operator, action.clone()).unwrap();
    assert!(matches!(&first.result, OrchestrationActionResult::TaskUnassigned {
        assignment_generation: 2, previous_run_id: Some(previous), duplicate: false, ..
    } if previous == &run));
    let bytes = fixture.state_bytes();
    let replay = fixture.apply(&operator, action).unwrap();
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
    caller_mut(&mut actor).conversation.as_mut().unwrap().session_id = "not/a/uuid:conversation".into();
    let action = OrchestrationAction::RunRegister { operation_id: id() };
    fixture.apply(&actor, action.clone()).unwrap();
    let bytes = fixture.state_bytes();
    fixture.apply(&actor, action.clone()).unwrap();
    assert_eq!(fixture.state_bytes(), bytes);
    let mut other = actor.clone();
    caller_mut(&mut other).conversation.as_mut().unwrap().session_id = "different-root-chat".into();
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

fn replacement_action(root: &str, task: &Task, run: &Run) -> OrchestrationAction {
    OrchestrationAction::WorkerReplace { operation_id: id(), root_id: root.into(), task_id: task.task_id.clone(),
        expected_run_id: run.run_id.clone(), expected_assignment_generation: run.assignment_generation.unwrap(),
        reason: "Continue safely from the committed checkpoint".into() }
}

fn replacement_input(fixture: &Fixture, run: &Run) -> ResolvedMutationInput {
    let project = run.project.clone().unwrap_or_else(|| fixture.project());
    let source = match &project.source {
        ProjectSource::Git { .. } => ReplacementSource::GitHead {
            commit: "a".repeat(40), source_workspace_id: Some(project.source_workspace_id.clone().unwrap_or_else(|| "source-space".into())),
        },
        ProjectSource::Directory { root } => ReplacementSource::Directory {
            root: root.clone(), source_workspace_id: project.source_workspace_id.clone(),
        },
    };
    ResolvedMutationInput { replacement_source: Some(source), checkout: run.checkout.clone(),
        project: Some(project), ..Default::default() }
}


#[test]
fn explicit_operator_replaces_pending_attempt_with_generation_fences() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Operator replacement");
    let _ = fixture.root_actor(&root);
    let worker = fixture.start(&root, &task);
    replacement_source(&fixture, &root, &worker);
    let old = fixture.run(&worker);
    let operator = Actor::Operator(OperatorOrigin::Native);
    let mut stale = replacement_action(&root, &task, &old);
    let OrchestrationAction::WorkerReplace { expected_assignment_generation, .. } = &mut stale else { unreachable!() };
    *expected_assignment_generation += 1;
    let bytes = fixture.state_bytes();
    assert_code(fixture.service.mutate_resolved(&operator, OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None, action: stale,
    }, &replacement_input(&fixture, &old)), "assignment_generation_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    let response = fixture.service.mutate_resolved(&operator, OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None, action: replacement_action(&root, &task, &old),
    }, &replacement_input(&fixture, &old)).unwrap();
    let OrchestrationActionResult::WorkerReplaced { start, .. } = response.result else { unreachable!() };
    assert_eq!(fixture.run(&worker).close_reason, Some(CloseReason::Superseded));
    assert_eq!(fixture.run(&start.run_id).task_id, Some(task.task_id));
    assert_eq!(serde_json::to_value(fixture.run(&worker).attachment).unwrap(),
        serde_json::to_value(old.attachment).unwrap(), "operator authority never stops the old writer");
}
fn replacement_source(fixture: &Fixture, root: &str, run: &str) {
    let source = fixture.root.join("borrowed-source");
    std::fs::create_dir_all(&source).unwrap();
    let (mut project, _) = project_identity::resolve_owned_directory(&source).unwrap();
    project.source_workspace_id = Some("source-space".into());
    fixture.seed_run(root, |run| run.project = Some(project.clone()));
    fixture.seed_run(run, |run| run.project = Some(project));
}

#[test]
fn credential_only_claim_activates_without_inventing_conversation_and_enriches_metadata_later() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Harness-neutral capability");
    let (worker, mut actor) = pending_worker(&fixture, &root, &task);
    caller_mut(&mut actor).conversation = None;
    let caller = match &actor { Actor::Agent(caller) => caller, _ => unreachable!() };
    fixture.service.ensure_registered(caller).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
    assert!(fixture.run(&worker).attachment.unwrap().conversation.is_none());
    assert!(ownership(&fixture, &root, &task).owner.unwrap().conversation.is_none());
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    let bytes = fixture.state_bytes();
    fixture.service.ensure_registered(caller).unwrap();
    assert_eq!(fixture.state_bytes(), bytes);
    caller_mut(&mut actor).conversation = Some(ConversationIdentity { harness: fixture.run(&worker).launch_config.harness_kind.clone(), session_id: "observed-real-session".into() });
    let caller = match &actor { Actor::Agent(caller) => caller, _ => unreachable!() };
    fixture.service.ensure_registered(caller).unwrap();
    assert_eq!(fixture.run(&worker).attachment.unwrap().conversation, caller.conversation);
    let brief_count = fixture.state().messages.iter().filter(|message| message.to_run_id == worker && message.kind == MessageKind::WorkBrief).count();
    assert_eq!(brief_count, 1);
    let bytes = fixture.state_bytes();
    caller_mut(&mut actor).conversation.as_mut().unwrap().session_id = "wrong-session".into();
    assert_code(fixture.apply(&actor, report(ReportKind::Progress, None, None)), "job_unauthorized");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn credential_only_root_can_schedule_without_actual_session_metadata() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut actor = fixture.neutral_actor(&root);
    caller_mut(&mut actor).conversation = None;
    fixture.service.ensure_registered(match &actor { Actor::Agent(caller) => caller, _ => unreachable!() }).unwrap();
    let task = fixture.create_task(&root, "Neutral supervisor");
    fixture.apply(&actor, schedule_action(&root, &task)).unwrap();
    assert!(ownership(&fixture, &root, &task).schedule.is_some());
}

#[test]
fn atomic_replacement_of_live_writer_retains_physical_history_and_rejects_every_old_authority() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Replace current writer");
    let supervisor = fixture.root_actor(&root);
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let actor = fixture.launch_and_bind(&worker);
    replacement_source(&fixture, &root, &worker);
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    let old = fixture.run(&worker);
    let claims = serde_json::to_value(&fixture.state().checkout_claims).unwrap();
    let action = replacement_action(&root, &task, &old);
    let resolved = replacement_input(&fixture, &old);
    let apply = |action| fixture.service.mutate_resolved(&supervisor,
        OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None, action }, &resolved);
    let response = apply(action.clone()).unwrap();
    let OrchestrationActionResult::WorkerReplaced { start, previous_run_id, assignment_generation, placement } = response.result else { unreachable!() };
    assert_eq!(previous_run_id, worker);
    assert_eq!(assignment_generation, 3);
    let expected_directory = fixture.service.base().join("attempts").join(&start.run_id).to_string_lossy().into_owned();
    assert_eq!(placement, ReplacementPlacement::IsolatedDirectory { directory: expected_directory });
    let successor = fixture.run(&start.run_id);
    assert_eq!(successor.attempt, old.attempt + 1);
    assert!(matches!(&successor.target, Some(DispatchTarget::OwnedAttemptDirectory { source_root, source_workspace_id: Some(space) })
        if Some(source_root) == old.project.as_ref().and_then(|project| match &project.source { ProjectSource::Directory { root } => Some(root), _ => None })
            && space == "source-space"));
    assert_eq!(successor.project, old.project);
    let retained = fixture.run(&worker);
    assert_eq!(retained.close_reason, Some(CloseReason::Superseded));
    assert_eq!(serde_json::to_value(&retained.attachment).unwrap(), serde_json::to_value(&old.attachment).unwrap());
    assert_eq!(serde_json::to_value(&retained.source_output).unwrap(), serde_json::to_value(&old.source_output).unwrap());
    assert_eq!(serde_json::to_value(&fixture.state().checkout_claims).unwrap(), claims);
    assert!(fixture.state().messages.iter().any(|message| matches!(&message.from, ActorRef::Run { run_id } if run_id == &worker)));
    assert!(fixture.state().credentials.iter().filter(|credential| credential.run_id == worker)
        .all(|credential| credential.state == credentials::CredentialState::Revoked));
    let bytes = fixture.state_bytes();
    let replay = apply(action.clone()).unwrap();
    assert!(matches!(replay.result, OrchestrationActionResult::WorkerReplaced { start: WorkerStartResult { duplicate: true, .. }, .. }));
    assert_eq!(fixture.state_bytes(), bytes);
    assert_code(fixture.apply(&actor, report(ReportKind::Progress, None, None)), "job_unauthorized");
    assert_code(fixture.apply(&actor, OrchestrationAction::RunRegister { operation_id: caller_auth(&actor).credential_id.clone() }), "job_unauthorized");
    assert_code(apply(replacement_action(&root, &task, &old)), "assignment_generation_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn pending_sticky_attempt_can_be_replaced_without_cleanup_or_late_launch_authority() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Replace pending attempt");
    let supervisor = fixture.root_actor(&root);
    let (worker, actor) = pending_worker(&fixture, &root, &task);
    replacement_source(&fixture, &root, &worker);
    fixture.seed_run(&worker, |run| {
        run.stage = RunStage::Starting;
        run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchUnknown;
        run.resume = Some(ResumeRecord { operation_id: id(), run_id: worker.clone(), previous_attachment_epoch: Some(1),
            phase: ResumePhase::Launching, cause: None, decision_id: None, replacement_attachment_epoch: None, updated_at: now() });
    });
    let old = fixture.run(&worker);
    let response = fixture.service.mutate_resolved(&supervisor, OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None, action: replacement_action(&root, &task, &old),
    }, &replacement_input(&fixture, &old)).unwrap();
    let OrchestrationActionResult::WorkerReplaced { start, assignment_generation, .. } = response.result else { unreachable!() };
    assert_eq!(assignment_generation, 3);
    assert_eq!(fixture.run(&start.run_id).assignment_generation, Some(3));
    assert!(fixture.service.record_dispatch(&worker, DispatchUpdate::Step { step: DispatchStep::LaunchPending, error: None }).is_err());
    assert!(fixture.apply(&actor, OrchestrationAction::RunRegister { operation_id: id() }).is_err());
    assert_eq!(serde_json::to_value(&fixture.run(&worker).attachment).unwrap(), serde_json::to_value(&old.attachment).unwrap());
}

#[test]
fn replacement_refusals_are_atomic_for_stale_generation_wrong_root_worker_and_unproven_source() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Replacement fences");
    let supervisor = fixture.root_actor(&root);
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let actor = fixture.launch_and_bind(&worker);
    replacement_source(&fixture, &root, &worker);
    let old = fixture.run(&worker);
    for mode in 0..4 {
        let mut action = replacement_action(&root, &task, &old);
        let mut resolved = replacement_input(&fixture, &old);
        let OrchestrationAction::WorkerReplace { expected_assignment_generation, root_id, .. } = &mut action else { unreachable!() };
        if mode == 0 { *expected_assignment_generation += 1; }
        if mode == 1 { *root_id = id(); }
        if mode == 3 { resolved.replacement_source = None; }
        let bytes = fixture.state_bytes();
        assert!(fixture.service.mutate_resolved(if mode == 2 { &actor } else { &supervisor },
            OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None, action }, &resolved).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
}

fn schedule_action(root: &str, task: &Task) -> OrchestrationAction {
    OrchestrationAction::TaskSchedule { operation_id: id(), root_id: root.into(), task_id: task.task_id.clone(),
        target: Some(target()), brief: "Execute the canonical task when prerequisites are accepted".into(),
        cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe, label: None,
        expected_task_revision: Some(task.task_revision.clone()), expected_doc_revision: None }
}

fn failed_scheduled_attempt() -> (Fixture, String, Task, Actor, ScheduleCandidate, WorkerStartResult) {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Correct placement");
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&supervisor, schedule_action(&root, &task)).unwrap();
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    let start = fixture.service.scheduled_start(&candidate, &ResolvedMutationInput {
        project: Some(fixture.project()), ..Default::default()
    }).unwrap().unwrap();
    fixture.service.record_dispatch(&start.run_id, DispatchUpdate::PlanFailed { error: ErrorResponse {
        code: "space_exists_for_path".into(), message: "Checkout already belongs to a Space".into(),
    } }).unwrap();
    (fixture, root, task, supervisor, candidate, start)
}

#[test]
fn explicit_schedule_correction_supersedes_only_failed_pending_authorization_and_dispatches_attempt_two() {
    for cancel_first in [false, true] {
        let (fixture, root, task, supervisor, stale_candidate, old) = failed_scheduled_attempt();
        fixture.seed_state(|state| {
            let run = state.runs.iter().find(|run| run.run_id == old.run_id).unwrap();
            state.credentials.push(credentials::test_job_credential(run, JobRole::Worker).0);
        });
        if cancel_first {
            fixture.apply(&supervisor, OrchestrationAction::TaskScheduleCancel {
                operation_id: id(), root_id: root.clone(), task_id: task.task_id.clone(),
            }).unwrap();
        }
        let mut correction = schedule_action(&root, &task);
        if let OrchestrationAction::TaskSchedule { target, expected_doc_revision, .. } = &mut correction {
            *target = Some(DispatchTarget::ExistingSpace { workspace_id: "corrected-source".into() });
            *expected_doc_revision = Some(task_document_revision(&fixture, &root));
        }
        fixture.apply(&supervisor, correction.clone()).unwrap();
        let closed = fixture.run(&old.run_id);
        assert_eq!(closed.stage, RunStage::Closed);
        assert_eq!(closed.close_reason, Some(CloseReason::Superseded));
        assert!(closed.setup.is_none());
        assert!(closed.checkout.is_none());
        assert!(fixture.state().credentials.iter().filter(|credential| credential.run_id == old.run_id)
            .all(|credential| credential.state == credentials::CredentialState::Revoked));
        assert_eq!(ownership(&fixture, &root, &task).generation, closed.assignment_generation.unwrap() + 1);
        let bytes = fixture.state_bytes();
        assert!(matches!(fixture.apply(&supervisor, correction).unwrap().result,
            OrchestrationActionResult::TaskScheduled { duplicate: true, .. }));
        assert_eq!(fixture.state_bytes(), bytes, "Exact replay cannot revoke a second generation");
        let resolved = ResolvedMutationInput { project: Some(fixture.project()), ..Default::default() };
        assert!(fixture.service.scheduled_start(&stale_candidate, &resolved).unwrap().is_none());
        let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
        let successor = fixture.service.scheduled_start(&candidate, &resolved).unwrap().unwrap();
        assert_eq!(successor.attempt, 2);
        assert_eq!(fixture.run(&successor.run_id).assignment_generation,
            Some(ownership(&fixture, &root, &task).generation + 1));
        fixture.setup(&successor.run_id);
        fixture.launch_and_bind(&successor.run_id);
        assert_eq!(ownership(&fixture, &root, &task).owner.unwrap().run_id, successor.run_id,
            "Consumer registration must not encounter the old logical assignment");
        assert_eq!(fixture.state().runs.iter().filter(|run| run.task_id.as_ref() == Some(&task.task_id)
            && run.stage != RunStage::Closed).count(), 1);
    }
}

#[test]
fn schedule_correction_preserves_started_unknown_and_ambiguous_attempts_atomically() {
    for boundary in ["started", "working", "setup", "checkout", "claim", "location", "process", "scope", "source_location", "launch_attempt",
        "latch", "move", "attachment", "generation", "owner", "orphan_claim", "duplicate",
        "session", "parent", "cancelled_unknown", "implicit", "task_revision", "doc_revision"] {
        let (fixture, root, task, supervisor, _, old) = failed_scheduled_attempt();
        let mut correction = schedule_action(&root, &task);
        match boundary {
            "started" => fixture.seed_run(&old.run_id, |run| run.dispatch.as_mut().unwrap().agent_started = true),
            "working" => fixture.seed_run(&old.run_id, |run| {
                run.stage = RunStage::Working;
                run.dispatch.as_mut().unwrap().step = DispatchStep::Launched;
            }),
            "source_location" => fixture.seed_run(&old.run_id, |run| {
                let mut attachment = run.attachment.as_ref().unwrap().clone();
                attachment.location = Some(RunLocation {
                    boot_id: None, peer_process: None, terminal_id: None, native_session_id: None,
                    endpoint_identity: "unknown".into(), session_id: SESSION.into(), workspace_id: "unknown".into(),
                    tab_id: "unknown".into(), pane_id: "unknown".into(), launch_tag: "unknown".into(),
                });
                run.dispatch.as_mut().unwrap().source_location = attachment.location;
            }),
            "launch_attempt" => fixture.seed_run(&old.run_id, |run| run.dispatch.as_mut().unwrap().launch_attempt = 2),
            "setup" => fixture.setup(&old.run_id),
            "checkout" => fixture.seed_run(&old.run_id, |run| run.checkout = Some(CheckoutIdentity {
                key: "unknown".into(), path: "/unknown".into(), git: None,
                directory_identity: DirectoryIdentity { device: "1".into(), inode: "2".into() },
            })),
            "claim" => fixture.seed_run(&old.run_id, |run| run.checkout_claim_id = Some(id())),
            "location" => fixture.seed_run(&old.run_id, |run| run.attachment.as_mut().unwrap().location = Some(RunLocation {
                boot_id: None, peer_process: None, terminal_id: None, native_session_id: None,
                endpoint_identity: "unknown".into(), session_id: SESSION.into(), workspace_id: "unknown".into(),
                tab_id: "unknown".into(), pane_id: "unknown".into(), launch_tag: "unknown".into(),
            })),
            "process" => fixture.seed_run(&old.run_id, |run| run.attachment.as_mut().unwrap().process = Some(NativeProcessIdentity {
                pid: 123, start_ticks: 1, kernel_boot_id: Some("boot".into()),
            })),
            "scope" => fixture.seed_run(&old.run_id, |run| run.dispatch.as_mut().unwrap().scope_unit = Some("unknown.scope".into())),
            "latch" => fixture.seed_run(&old.run_id, |run| run.dispatch.as_mut().unwrap().latch_state = Some(AgentLatchState::ReleaseIntent)),
            "move" => fixture.seed_run(&old.run_id, |run| run.dispatch.as_mut().unwrap().move_operation_id = Some(id())),
            "attachment" => fixture.seed_run(&old.run_id, |run| run.attachment.as_mut().unwrap().state = AttachmentState::Connecting),
            "generation" => fixture.seed_state(|state| {
                state.task_ownership.iter_mut().find(|record| record.task_id == task.task_id).unwrap().generation += 1;
            }),
            "owner" => fixture.seed_state(|state| {
                state.task_ownership.iter_mut().find(|record| record.task_id == task.task_id).unwrap().owner =
                    Some(TaskOwner { run_id: old.run_id.clone(), conversation: None, claim_operation_id: id() });
            }),
            "orphan_claim" => fixture.seed_state(|state| {
                let run = state.runs.iter().find(|run| run.run_id == old.run_id).unwrap();
                let attachment = run.attachment.as_ref().unwrap();
                state.checkout_claims.push(claims::CheckoutClaim { claim_id: id(), revision: 1,
                    state: claims::ClaimState::Reserved, transition_operation_id: None,
                    checkout: CheckoutIdentity { key: "unknown".into(), path: "/unknown".into(), git: None,
                        directory_identity: DirectoryIdentity { device: "1".into(), inode: "2".into() } },
                    holder: Some(AttachmentReference { run_id: old.run_id.clone(),
                        attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch }) });
            }),
            "session" => fixture.seed_run(&old.run_id, |run| run.session_id = "foreign".into()),
            "parent" => fixture.seed_run(&old.run_id, |run| run.parent_run_id = Some(id())),
            "cancelled_unknown" => {
                fixture.seed_run(&old.run_id, |run| run.dispatch.as_mut().unwrap().agent_started = true);
                fixture.apply(&supervisor, OrchestrationAction::TaskScheduleCancel {
                    operation_id: id(), root_id: root.clone(), task_id: task.task_id.clone(),
                }).unwrap();
            },
            "duplicate" => fixture.seed_state(|state| {
                let mut extra = state.runs.iter().find(|run| run.run_id == old.run_id).unwrap().clone();
                extra.run_id = id();
                state.runs.push(extra);
            }),
            "implicit" => if let OrchestrationAction::TaskSchedule { target, .. } = &mut correction { *target = None; },
            "task_revision" => if let OrchestrationAction::TaskSchedule { expected_task_revision, .. } = &mut correction { *expected_task_revision = Some("stale".into()); },
            "doc_revision" => if let OrchestrationAction::TaskSchedule { expected_doc_revision, .. } = &mut correction { *expected_doc_revision = Some("stale".into()); },
            _ => unreachable!(),
        }
        let bytes = fixture.state_bytes();
        assert!(fixture.apply(&supervisor, correction).is_err(), "{boundary}");
        assert_eq!(fixture.state_bytes(), bytes, "{boundary}: refusal cannot erase unknown authority or overwrite schedule");
    }
}

#[test]
fn schedule_normalization_cannot_override_explicit_space_or_unverified_source() {
    for mismatch in ["explicit_space", "directory", "wrong_space", "missing_repository", "retained_checkout", "changed_authorized_path"] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Normalization fence");
        let supervisor = fixture.root_actor(&root);
        let mut action = schedule_action(&root, &task);
        if mismatch != "explicit_space"
            && let OrchestrationAction::TaskSchedule { target, .. } = &mut action {
            *target = Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
                path: fixture.root.to_string_lossy().into_owned(), label: None, task_name: None, focus: false,
            } });
        }
        fixture.apply(&supervisor, action).unwrap();
        let mut candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
        let mut project = fixture.project();
        project.source = ProjectSource::Git { common_dir: "/git/common".into() };
        project.repository_id = Some("configured".into());
        project.source_workspace_id = Some("source".into());
        let mut resolved = ResolvedMutationInput { project: Some(project), normalized_target: Some(DispatchTarget::SpaceWorktree {
            workspace_id: "source".into(), branch: None, base_ref: None,
        }), ..Default::default() };
        match mismatch {
            "directory" => resolved.project = Some(fixture.project()),
            "wrong_space" => resolved.project.as_mut().unwrap().source_workspace_id = Some("foreign".into()),
            "missing_repository" => resolved.project.as_mut().unwrap().repository_id = None,
            "retained_checkout" => resolved.checkout = Some(CheckoutIdentity { key: "borrowed".into(),
                path: "/borrowed".into(), git: None, directory_identity: DirectoryIdentity { device: "1".into(), inode: "2".into() } }),
            "changed_authorized_path" => if let Some(DispatchTarget::Setup {
                request: WorkspaceSetupRequest::Open { path, .. },
            }) = &mut candidate.schedule.target { *path = "/foreign-request".into(); },
            _ => {},
        }
        let bytes = fixture.state_bytes();
        assert_eq!(fixture.service.scheduled_start(&candidate, &resolved).unwrap_err().code, "schedule_source_unresolved");
        assert_eq!(fixture.state_bytes(), bytes, "{mismatch}");
    }
}

#[test]
fn dependency_schedule_queues_until_acceptance_rechecks_fresh_graph_and_is_idempotent() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let prerequisite = fixture.create_task(&root, "Prerequisite");
    let dependent = fixture.create_task(&root, "Dependent");
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&supervisor, OrchestrationAction::TaskDependenciesSet { root_id: root.clone(),
        task_id: dependent.task_id.clone(), expected_task_revision: dependent.task_revision.clone(),
        expected_doc_revision: task_document_revision(&fixture, &root), depends_on: vec![prerequisite.task_id.clone()] }).unwrap();
    let dependent = fixture.task(&root, &dependent.task_id);
    let schedule = schedule_action(&root, &dependent);
    fixture.apply(&supervisor, schedule.clone()).unwrap();
    let bytes = fixture.state_bytes();
    assert!(matches!(fixture.apply(&supervisor, schedule).unwrap().result, OrchestrationActionResult::TaskScheduled { duplicate: true, .. }));
    assert_eq!(fixture.state_bytes(), bytes);
    assert!(fixture.service.schedule_candidates(4).unwrap().is_empty());
    {
        let locked = fixture.service.store.lock().unwrap();
        locked.tasks(&root).unwrap().check(&prerequisite.task_id, &prerequisite.task_revision, true).unwrap();
    }
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    let resolved = ResolvedMutationInput { project: Some(fixture.project()), ..Default::default() };
    let start = fixture.service.scheduled_start(&candidate, &resolved).unwrap().unwrap();
    assert_eq!(start.attempt, 1);
    assert!(ownership(&fixture, &root, &dependent).schedule.is_some());
    let bytes = fixture.state_bytes();
    assert!(fixture.service.scheduled_start(&candidate, &resolved).unwrap().unwrap().duplicate);
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.apply(&supervisor, OrchestrationAction::TaskScheduleCancel { operation_id: id(), root_id: root.clone(),
        task_id: dependent.task_id.clone() }).unwrap();
    assert!(ownership(&fixture, &root, &dependent).schedule.is_none());
    assert_eq!(fixture.run(&start.run_id).stage, RunStage::Queued);
    assert!(fixture.service.scheduled_start(&candidate, &resolved).unwrap().is_none());
}

#[test]
fn schedule_dispatch_refuses_unbound_root_without_discarding_durable_authorization() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Fenced scheduled task");
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&supervisor, schedule_action(&root, &task)).unwrap();
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    fixture.seed_state(|state| {
        for credential in &mut state.credentials {
            if credential.run_id == root { credential.state = credentials::CredentialState::Revoked; }
        }
    });
    let bytes = fixture.state_bytes();
    assert!(fixture.service.schedule_candidates(4).unwrap().is_empty());
    assert!(fixture.service.scheduled_start(&candidate, &ResolvedMutationInput {
        project: Some(fixture.project()), ..Default::default()
    }).unwrap().is_none());
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn durable_task_schedule_survives_liaison_attachment_and_credential_rotation() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Resume authorized schedule");
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&supervisor, schedule_action(&root, &task)).unwrap();
    let original = ownership(&fixture, &root, &task).schedule.unwrap();
    fixture.seed_state(|state| {
        for credential in &mut state.credentials {
            if credential.run_id == root { credential.state = credentials::CredentialState::Revoked; }
        }
        let run = state.runs.iter_mut().find(|run| run.run_id == root).unwrap();
        let attachment = run.attachment.as_mut().unwrap();
        attachment.epoch += 1;
        attachment.attachment_id = id();
        attachment.state = AttachmentState::Reserved;
        attachment.conversation = None;
    });
    let mut restarted = fixture.neutral_actor(&root);
    caller_mut(&mut restarted).conversation = None;
    fixture.service.ensure_registered(match &restarted { Actor::Agent(caller) => caller, _ => unreachable!() }).unwrap();
    let candidates = fixture.service.schedule_candidates(4).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].schedule.principal_key, original.principal_key);
    let start = fixture.service.scheduled_start(&candidates[0], &ResolvedMutationInput {
        project: Some(fixture.project()), ..Default::default()
    }).unwrap().unwrap();
    assert_eq!(start.attempt, 1);
    assert!(fixture.apply(&supervisor, OrchestrationAction::TaskScheduleCancel {
        operation_id: id(), root_id: root.clone(), task_id: task.task_id.clone(),
    }).is_err());
}

#[test]
fn concurrent_replacements_publish_only_one_successor_and_one_generation_fence() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Concurrent replacement");
    let supervisor = fixture.root_actor(&root);
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let _actor = fixture.launch_and_bind(&worker);
    replacement_source(&fixture, &root, &worker);
    let old = fixture.run(&worker);
    let resolved = replacement_input(&fixture, &old);
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let apply = || {
            let action = replacement_action(&root, &task, &old);
            barrier.wait();
            fixture.service.mutate_resolved(&supervisor, OrchestrationMutationRequest {
                session_id: SESSION.into(), expected_revision: None, action,
            }, &resolved)
        };
        let first = scope.spawn(apply);
        let second = scope.spawn(apply);
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(results.0.is_ok(), results.1.is_ok());
    let state = fixture.state();
    assert_eq!(state.runs.iter().filter(|run| run.task_id.as_ref() == Some(&task.task_id) && run.stage != RunStage::Closed).count(), 1);
    assert_eq!(ownership(&fixture, &root, &task).generation, 2);
}

#[test]
fn newly_blocked_dependency_and_parallel_limit_are_rechecked_at_schedule_claim() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Claim recheck");
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&supervisor, schedule_action(&root, &task)).unwrap();
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    let blocker = fixture.create_task(&root, "New prerequisite");
    fixture.apply(&supervisor, OrchestrationAction::TaskDependenciesSet {
        root_id: root.clone(), task_id: task.task_id.clone(), expected_task_revision: task.task_revision.clone(),
        expected_doc_revision: task_document_revision(&fixture, &root), depends_on: vec![blocker.task_id.clone()],
    }).unwrap();
    let bytes = fixture.state_bytes();
    assert!(fixture.service.scheduled_start(&candidate, &ResolvedMutationInput { project: Some(fixture.project()), ..Default::default() }).unwrap().is_none());
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.seed_run(&root, |run| run.launch_config.max_parallel = 0);
    assert!(fixture.service.schedule_candidates(4).unwrap().is_empty());
}

#[test]
fn precise_exit_proof_replaces_in_same_checkout_without_overwriting_old_attachment() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Proven same-checkout replacement");
    let supervisor = fixture.root_actor(&root);
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let _actor = fixture.launch_and_bind(&worker);
    let old = fixture.run(&worker);
    let attachment = old.attachment.as_ref().unwrap();
    let claim = fixture.state().checkout_claims.into_iter().find(|claim| old.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    let proof = claims::WriterExitProof { incarnation: AttachmentReference {
        run_id: worker.clone(), attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch,
    }, claim_revision: claim.revision, checkout: old.checkout.clone(), machine_id: attachment.machine_id.clone().unwrap(),
        previous_kernel_boot_id: attachment.process.as_ref().unwrap().kernel_boot_id.clone().unwrap(),
        observed_kernel_boot_id: "kernel-two".into(), all_consumers_absent: true, basis: claims::ExitBasis::Reboot };
    let response = fixture.service.mutate_resolved(&supervisor, OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None, action: replacement_action(&root, &task, &old),
    }, &ResolvedMutationInput { writer_exit_proof: Some(proof), ..Default::default() }).unwrap();
    let OrchestrationActionResult::WorkerReplaced { start, placement, .. } = response.result else { unreachable!() };
    assert_eq!(placement, ReplacementPlacement::SameCheckout);
    let successor = fixture.run(&start.run_id);
    assert_eq!(successor.checkout, old.checkout);
    assert_eq!(successor.checkout_claim_id, old.checkout_claim_id);
    assert_eq!(successor.stage, RunStage::Starting);
    assert_eq!(serde_json::to_value(&fixture.run(&worker).attachment).unwrap(), serde_json::to_value(&old.attachment).unwrap());
    let claim = fixture.state().checkout_claims.into_iter().find(|claim| successor.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    assert_eq!(claim.holder.unwrap().run_id, successor.run_id);
}

#[test]
fn first_credentialed_call_can_claim_before_physical_receipt_without_late_stage_reset() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Fast neutral harness");
    let _supervisor = fixture.root_actor(&root);
    let worker = fixture.start(&root, &task);
    let mut actor = fixture.launch_tab(&worker);
    let physical = fixture.run(&worker).attachment.unwrap();
    fixture.seed_run(&worker, |run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.location = None;
        attachment.process = None;
        attachment.shell = None;
        attachment.state = AttachmentState::Connecting;
        run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchIntent;
    });
    caller_mut(&mut actor).conversation = None;
    fixture.service.ensure_registered(match &actor { Actor::Agent(caller) => caller, _ => unreachable!() }).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
    let generation = ownership(&fixture, &root, &task).generation;
    fixture.service.record_dispatch(&worker, DispatchUpdate::TabReceipt {
        location: physical.location.unwrap(), launch_shell_identity: physical.shell, process_scope: None,
    }).unwrap();
    let receipt = fixture.run(&worker);
    assert_eq!(receipt.stage, RunStage::Working);
    assert_eq!(ownership(&fixture, &root, &task).generation, generation);
    assert!(receipt.attachment.as_ref().unwrap().conversation.is_none());
    let bytes = fixture.state_bytes();
    fixture.service.record_launch_pending(&receipt).unwrap();
    assert_eq!(fixture.state_bytes(), bytes);
}

fn latched_staging_worker(fixture: &Fixture) -> (String, Task, String, Run) {
    let root = fixture.root();
    let task = fixture.create_task(&root, "One terminal placement");
    let _supervisor = fixture.root_actor(&root);
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let _actor = fixture.launch_tab(&worker);
    fixture.seed_run(&worker, |run| {
        let source = run.attachment.as_ref().unwrap().location.as_ref().unwrap();
        let dispatch = run.dispatch.as_mut().unwrap();
        dispatch.latch = Some(AgentLaunchReceipt { latch: "/private/test.fifo".into(), nonce: "nonce".into(),
            latch_device: "1".into(), latch_inode: "2".into() });
        dispatch.latch_state = Some(AgentLatchState::Awaiting);
        dispatch.staging_workspace_id = Some(source.workspace_id.clone());
        let setup = run.setup.as_mut().unwrap();
        setup.operation_id = Some(id());
        setup.generation = Some(1);
        setup.ownership = Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree);
    });
    let staged = fixture.run(&worker);
    (root, task, worker, staged)
}

fn moved_receipts(staged: &Run) -> (RunLocation, SetupSummary) {
    let mut location = staged.attachment.as_ref().unwrap().location.clone().unwrap();
    location.workspace_id = "actual-owned-space".into();
    location.tab_id = "actual-moved-tab".into();
    location.pane_id = "actual-moved-pane".into();
    let mut setup = staged.setup.clone().unwrap();
    setup.workspace_id = Some(location.workspace_id.clone());
    (location, setup)
}

#[test]
fn durable_unknown_move_reconciles_exact_terminal_but_never_resends() {
    let fixture = Fixture::new();
    let (_root, _task, worker, staged) = latched_staging_worker(&fixture);
    let source = staged.attachment.as_ref().unwrap().location.clone().unwrap();
    let move_id = id();
    assert!(fixture.service.record_move_intent(&staged, source.clone(), &move_id).unwrap());
    let intent = fixture.run(&worker);
    let bytes = fixture.state_bytes();
    assert!(!fixture.service.record_move_intent(&staged, source.clone(), &move_id).unwrap());
    assert_eq!(fixture.state_bytes(), bytes);
    assert!(fixture.service.record_move_unknown(&intent, ErrorResponse { code: "move_outcome_unknown".into(), message: "Adapter response was lost".into() }).unwrap());
    let unknown = fixture.run(&worker);
    let bytes = fixture.state_bytes();
    assert!(!fixture.service.record_move_intent(&unknown, source.clone(), &id()).unwrap());
    assert_eq!(fixture.state_bytes(), bytes);
    let (location, setup) = moved_receipts(&staged);
    assert!(fixture.service.record_placement(&unknown, location.clone(), setup).unwrap());
    let placed = fixture.run(&worker);
    assert_eq!(placed.dispatch.as_ref().unwrap().step, DispatchStep::Placed);
    assert_eq!(placed.dispatch.as_ref().unwrap().source_location.as_ref().unwrap().workspace_id, source.workspace_id);
    assert_eq!(placed.attachment.as_ref().unwrap().location.as_ref().unwrap().terminal_id, source.terminal_id);
    assert_eq!(placed.attachment.as_ref().unwrap().location.as_ref().unwrap().workspace_id, location.workspace_id);
    let bytes = fixture.state_bytes();
    assert!(fixture.service.record_dispatch(&worker, DispatchUpdate::TabReceipt {
        location: source, launch_shell_identity: staged.attachment.as_ref().unwrap().shell.clone(), process_scope: None,
    }).is_err());
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn moved_placement_refuses_foreign_terminal_changed_checkout_and_superseded_epoch() {
    let fixture = Fixture::new();
    let (root, task, worker, staged) = latched_staging_worker(&fixture);
    let source = staged.attachment.as_ref().unwrap().location.clone().unwrap();
    assert!(fixture.service.record_move_intent(&staged, source, &id()).unwrap());
    let intent = fixture.run(&worker);
    for mode in 0..3 {
        let (mut location, mut setup) = moved_receipts(&staged);
        if mode == 0 { location.terminal_id = Some("foreign-terminal".into()); }
        if mode == 1 { setup.checkout_path.push_str("-foreign"); }
        if mode == 2 { setup.operation_id = Some(id()); }
        let bytes = fixture.state_bytes();
        assert!(fixture.service.record_placement(&intent, location, setup).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&supervisor, unassign(&root, &task, 0)).unwrap();
    let (location, setup) = moved_receipts(&staged);
    let bytes = fixture.state_bytes();
    assert!(!fixture.service.record_placement(&intent, location, setup).unwrap());
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn latch_effect_intent_is_at_most_once_except_proven_reader_absence_and_preserves_fast_registration() {
    let fixture = Fixture::new();
    let (_root, _task, worker, staged) = latched_staging_worker(&fixture);
    let source = staged.attachment.as_ref().unwrap().location.clone().unwrap();
    assert!(fixture.service.record_move_intent(&staged, source, &id()).unwrap());
    let (location, setup) = moved_receipts(&staged);
    assert!(fixture.service.record_placement(&fixture.run(&worker), location, setup).unwrap());
    let placed = fixture.run(&worker);
    let intent = fixture.service.record_latch_release_intent(&placed, None).unwrap().unwrap();
    let bytes = fixture.state_bytes();
    assert!(fixture.service.record_latch_release_intent(&intent, None).unwrap().is_none());
    assert_eq!(fixture.state_bytes(), bytes);
    assert!(fixture.service.record_latch_reader_absent(&intent).unwrap());
    let second_intent = fixture.service.record_latch_release_intent(&fixture.run(&worker), None).unwrap().unwrap();
    assert!(fixture.service.record_latch_release_unknown(&second_intent, ErrorResponse {
        code: "latch_write_unknown".into(), message: "Write completion unavailable".into(),
    }).unwrap());
    let bytes = fixture.state_bytes();
    assert!(fixture.service.record_latch_release_intent(&fixture.run(&worker), None).unwrap().is_none());
    assert_eq!(fixture.state_bytes(), bytes);

    let fixture = Fixture::new();
    let (root, task, worker, staged) = latched_staging_worker(&fixture);
    let source = staged.attachment.as_ref().unwrap().location.clone().unwrap();
    fixture.service.record_move_intent(&staged, source, &id()).unwrap();
    let (location, setup) = moved_receipts(&staged);
    fixture.service.record_placement(&fixture.run(&worker), location, setup).unwrap();
    let intent = fixture.service.record_latch_release_intent(&fixture.run(&worker), None).unwrap().unwrap();
    let mut actor = fixture.neutral_actor(&worker);
    caller_mut(&mut actor).conversation = None;
    fixture.service.ensure_registered(match &actor { Actor::Agent(caller) => caller, _ => unreachable!() }).unwrap();
    assert!(fixture.service.record_latch_released(&intent).unwrap());
    let final_run = fixture.run(&worker);
    assert_eq!(final_run.stage, RunStage::Working);
    assert_eq!(final_run.dispatch.as_ref().unwrap().step, DispatchStep::Launched);
    assert_eq!(final_run.dispatch.as_ref().unwrap().latch_state, Some(AgentLatchState::Released));
    assert_eq!(ownership(&fixture, &root, &task).owner.unwrap().run_id, worker);
    let bytes = fixture.state_bytes();
    assert!(!fixture.service.record_latch_released(&intent).unwrap());
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn root_placement_sets_only_actual_project_space_and_preserves_explicit_source() {
    for explicit in [None, Some("explicit-project-space")] {
        let fixture = Fixture::new();
        let root = fixture.root();
        fixture.setup(&root);
        let _actor = fixture.launch_tab(&root);
        fixture.seed_run(&root, |run| {
            run.project.as_mut().unwrap().source_workspace_id = explicit.map(str::to_owned);
            let staging = run.attachment.as_ref().unwrap().location.as_ref().unwrap().workspace_id.clone();
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.latch = Some(AgentLaunchReceipt { latch: "/private/root.fifo".into(), nonce: "root-nonce".into(),
                latch_device: "1".into(), latch_inode: "2".into() });
            dispatch.latch_state = Some(AgentLatchState::Awaiting);
            dispatch.staging_workspace_id = Some(staging);
            let setup = run.setup.as_mut().unwrap();
            setup.operation_id = Some(id());
            setup.generation = Some(1);
            setup.ownership = Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree);
        });
        let staged = fixture.run(&root);
        let source = staged.attachment.as_ref().unwrap().location.clone().unwrap();
        fixture.service.record_move_intent(&staged, source.clone(), &id()).unwrap();
        let (location, setup) = moved_receipts(&staged);
        fixture.service.record_placement(&fixture.run(&root), location.clone(), setup).unwrap();
        let placed = fixture.run(&root);
        assert_eq!(placed.project.as_ref().unwrap().source_workspace_id.as_deref(), Some(explicit.unwrap_or(&location.workspace_id)));
        assert_eq!(placed.dispatch.as_ref().unwrap().staging_workspace_id.as_deref(), Some(source.workspace_id.as_str()));
        assert_ne!(placed.attachment.as_ref().unwrap().location.as_ref().unwrap().workspace_id, source.workspace_id);
    }
}

#[test]
fn accepted_dependency_outputs_are_observed_without_turning_reads_into_dispatch_gates() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (parent, parent_run, worker) = fixture.working(&root);
    let child = fixture.create_task(&root, "Read prerequisite outputs");
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&supervisor, OrchestrationAction::TaskDependenciesSet {
        root_id: root.clone(), task_id: child.task_id.clone(),
        expected_task_revision: child.task_revision.clone(),
        expected_doc_revision: task_document_revision(&fixture, &root),
        depends_on: vec![parent.task_id.clone()],
    }).unwrap();
    let revision = task_document_revision(&fixture, &root);
    assert!(fixture.service.accepted_dependency_run_ids(SESSION, &root, &child.task_id, &revision).unwrap().is_empty());
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    fixture.apply(&supervisor, OrchestrationAction::Accept {
        run_id: parent_run.clone(), expected_task_revision: parent.task_revision.clone(),
    }).unwrap();
    let revision = task_document_revision(&fixture, &root);
    assert_eq!(fixture.service.accepted_dependency_run_ids(SESSION, &root, &child.task_id, &revision).unwrap(), vec![parent_run.clone()]);
    let child = fixture.task(&root, &child.task_id);
    fixture.apply(&supervisor, schedule_action(&root, &child)).unwrap();
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    let snapshot = fixture.service.schedule_input_requirements(&candidate).unwrap();
    assert_eq!(snapshot.accepted_sources[0].run_id, parent_run);
    assert!(matches!(snapshot.accepted_sources[0].source_output, SourceOutput::NonCode { .. }));
    assert!(fixture.service.scheduled_start(&candidate, &ResolvedMutationInput {
        project: Some(fixture.project()), ..Default::default()
    }).unwrap().is_some(), "Accepted non-code output does not invent a Git ancestry prerequisite");
}

#[test]
fn same_repository_code_input_is_pinned_and_rechecked_inside_schedule_publication() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (parent, parent_run, worker) = fixture.working(&root);
    let supervisor = fixture.root_actor(&root);
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    fixture.apply(&supervisor, OrchestrationAction::Accept {
        run_id: parent_run.clone(), expected_task_revision: parent.task_revision.clone(),
    }).unwrap();
    // This consumer fixture exercises the domain's host-evidence fence, not
    // Git execution. Main's live scenario proves actual commit ancestry.
    let project = fixture.project();
    fixture.seed_run(&parent_run, |run| {
        let result = run.result.as_mut().unwrap();
        let output = SourceOutput::Code {
            repository_key: project.key.clone(), checkout_key: "accepted-checkout".into(),
            base_commit: "a".repeat(40), head_commit: "b".repeat(40),
            commits: vec!["b".repeat(40)], task_revision: parent.task_revision.clone(),
            result_message_id: result.message_id.clone(), focused_checks: vec![],
        };
        result.source_output = Some(output.clone());
        run.source_output = Some(output);
    });
    let child = fixture.create_task(&root, "Consume actual code output");
    fixture.apply(&supervisor, OrchestrationAction::TaskDependenciesSet {
        root_id: root.clone(), task_id: child.task_id.clone(),
        expected_task_revision: child.task_revision.clone(),
        expected_doc_revision: task_document_revision(&fixture, &root),
        depends_on: vec![parent.task_id.clone()],
    }).unwrap();
    let child = fixture.task(&root, &child.task_id);
    let mut action = schedule_action(&root, &child);
    if let OrchestrationAction::TaskSchedule { target, .. } = &mut action {
        *target = Some(DispatchTarget::SpaceWorktree {
            workspace_id: "input-project-space".into(), branch: None, base_ref: None,
        });
    }
    fixture.apply(&supervisor, action).unwrap();
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    let snapshot = fixture.service.schedule_input_requirements(&candidate).unwrap();
    let mut resolved = ResolvedMutationInput { project: Some(project), ..Default::default() };
    let before = fixture.state_bytes();
    assert_eq!(fixture.service.scheduled_start(&candidate, &resolved).unwrap_err().code, "dependency_input_unavailable");
    assert_eq!(fixture.state_bytes(), before, "An unmet materialized input cannot publish an attempt");
    resolved.schedule_inputs = Some(ScheduleInputFacts {
        snapshot, base_commit: Some("c".repeat(40)), verified_code_runs: vec![parent_run.clone()],
    });
    fixture.seed_run(&parent_run, |run| {
        for output in [&mut run.source_output, &mut run.result.as_mut().unwrap().source_output] {
            if let Some(SourceOutput::Code { head_commit, .. }) = output { *head_commit = "d".repeat(40); }
        }
    });
    let before = fixture.state_bytes();
    assert_eq!(fixture.service.scheduled_start(&candidate, &resolved).unwrap_err().code, "dependency_input_changed");
    assert_eq!(fixture.state_bytes(), before);
    resolved.schedule_inputs.as_mut().unwrap().snapshot = fixture.service.schedule_input_requirements(&candidate).unwrap();
    let started = fixture.service.scheduled_start(&candidate, &resolved).unwrap().unwrap();
    assert!(matches!(fixture.run(&started.run_id).target,
        Some(DispatchTarget::SpaceWorktree { base_ref: Some(commit), .. }) if commit == "c".repeat(40)));
}

#[test]
fn resumed_worker_missing_space_rebinds_exact_epoch_without_promoting_borrowed_checkout() {
    for borrowed in [false, true] {
        for authorized in [false, true] {
            let fixture = Fixture::new();
            let (_, _, worker, _) = latched_staging_worker(&fixture);
            let operation = id();
            let original_creator = if borrowed { None } else { Some(id()) };
            let new_attachment_id = id();
            fixture.seed_state(|state| {
                for credential in state.credentials.iter_mut().filter(|credential| credential.run_id == worker) {
                    credential.state = credentials::CredentialState::Revoked;
                }
                for claim in state.checkout_claims.iter_mut().filter(|claim|
                    claim.holder.as_ref().is_some_and(|holder| holder.run_id == worker)) {
                    let holder = claim.holder.as_mut().unwrap();
                    holder.attachment_id = new_attachment_id.clone();
                    holder.attachment_epoch = 2;
                    claim.transition_operation_id = Some(operation.clone());
                }
                let run = state.runs.iter_mut().find(|run| run.run_id == worker).unwrap();
                run.attachment.as_mut().unwrap().epoch = 2;
                run.attachment.as_mut().unwrap().attachment_id = new_attachment_id.clone();
                let setup = run.setup.as_mut().unwrap();
                setup.operation_id = original_creator.clone();
                setup.generation = if borrowed { None } else { Some(1) };
                setup.workspace_id = Some("missing-retained-space".into());
                setup.ownership = if borrowed { None } else {
                    Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree)
                };
                if authorized {
                    run.resume = Some(ResumeRecord { operation_id: operation.clone(), run_id: worker.clone(),
                        previous_attachment_epoch: Some(1), replacement_attachment_epoch: Some(2),
                        phase: ResumePhase::Launching, cause: None, decision_id: None, updated_at: now() });
                    let at = now();
                    state.operations.push(store::DurableOperationRecord {
                        operation_id: operation.clone(), actor_run_id: Some(run.root_id.clone()),
                        principal_key: "authorized-resume-fixture".into(), root_id: run.root_id.clone(),
                        kind: store::DurableOperationKind::RunResume, request_digest: "exact-cutover-fixture".into(),
                        run_id: Some(worker.clone()), resume_operation_id: Some(operation.clone()),
                        result: None, effect_receipts: Vec::new(), created_at: at.clone(), updated_at: at,
                    });
                }
            });
            let staged = fixture.run(&worker);
            if authorized {
                assert!(fixture.service.resume_startup_pending(&staged).unwrap());
                let mut observed = staged.attachment.clone().unwrap();
                observed.state = AttachmentState::Connected;
                for wrong in 0..4 {
                    let mut foreign = observed.clone();
                    match wrong {
                        0 => foreign.process.as_mut().unwrap().pid += 1,
                        1 => foreign.shell.as_mut().unwrap().argv_digest = "foreign-shell".into(),
                        2 => foreign.machine_id = Some("foreign-machine".into()),
                        _ => foreign.epoch += 1,
                    }
                    let before = fixture.state_bytes();
                    assert_eq!(fixture.service.advance_resume_observed(&worker, &operation,
                        &ResolvedMutationInput { attachment_observation: Some(foreign), ..Default::default() })
                        .unwrap_err().code, "attachment_conflict");
                    assert_eq!(fixture.state_bytes(), before);
                }
                let before = fixture.state_bytes();
                let waiting = fixture.service.advance_resume_observed(&worker, &operation,
                    &ResolvedMutationInput { attachment_observation: Some(observed), ..Default::default() }).unwrap();
                assert!(waiting.duplicate);
                assert_eq!(waiting.resume.phase, ResumePhase::Launching);
                assert_eq!(fixture.state_bytes(), before, "Waiting FIFO presence is not failed semantic registration");
            }
            let source = staged.attachment.as_ref().unwrap().location.clone().unwrap();
            let move_operation = id();
            fixture.service.record_move_intent(&staged, source, &move_operation).unwrap();
            let (location, setup) = moved_receipts(&staged);
            let before = fixture.state_bytes();
            let outcome = fixture.service.record_placement(&fixture.run(&worker), location.clone(), setup);
            if authorized {
                assert!(outcome.unwrap());
                let placed = fixture.run(&worker);
                assert_eq!(placed.setup.as_ref().unwrap().operation_id, original_creator);
                assert_eq!(placed.setup.as_ref().unwrap().ownership, staged.setup.as_ref().unwrap().ownership);
                assert_eq!(placed.setup.as_ref().unwrap().workspace_id.as_ref(), Some(&location.workspace_id));
                assert_eq!(placed.checkout, staged.checkout);
                assert_eq!(placed.project, staged.project);
                assert_eq!(placed.dispatch.as_ref().unwrap().move_operation_id.as_ref(), Some(&move_operation));
                let intent = fixture.service.record_latch_release_intent(&placed, None).unwrap().unwrap();
                fixture.service.record_latch_released(&intent).unwrap();
                let released = fixture.run(&worker);
                let mut observed = released.attachment.clone().unwrap();
                observed.state = AttachmentState::Connected;
                let before = fixture.state_bytes();
                assert!(fixture.service.advance_resume_observed(&worker, &operation,
                    &ResolvedMutationInput { attachment_observation: Some(observed), ..Default::default() })
                    .unwrap().duplicate);
                assert_eq!(fixture.state_bytes(), before, "The released-to-first-CLI claim window is still startup");
                let mut caller = fixture.neutral_actor(&worker);
                caller_mut(&mut caller).conversation = None;
                // Ordinary CLI calls run CliRuntime::open's private-credential
                // claim before the low-level mutation. Fixture::apply alone
                // intentionally exercises that low-level authority boundary.
                let Actor::Agent(agent) = &caller else { unreachable!() };
                fixture.service.job_registration_run(agent).unwrap();
                fixture.service.ensure_registered(agent).unwrap();
                fixture.apply(&caller, report(ReportKind::Progress, None, None)).unwrap();
                assert_eq!(fixture.run(&worker).resume.as_ref().unwrap().phase, ResumePhase::Connected);
                assert_eq!(fixture.run(&worker).stage, RunStage::Working);
            } else {
                assert_eq!(outcome.unwrap_err().code, "placement_receipt_conflict");
                assert_eq!(fixture.state_bytes(), before);
            }
        }
    }
}
