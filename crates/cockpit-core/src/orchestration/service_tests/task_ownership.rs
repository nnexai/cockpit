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
