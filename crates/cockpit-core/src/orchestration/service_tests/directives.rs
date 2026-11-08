use super::*;

#[test]
fn directives_deliver_replay_scope_and_remove() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "First");
    let other = fixture.create_task(&root, "Second");
    let worker = fixture.start(&root, &task);
    let sibling = fixture.start(&root, &other);
    let directive_id = id();
    let action = OrchestrationAction::DirectiveAdd {
        directive_id: directive_id.clone(), root_id: root.clone(),
        scope: DirectiveScope::Task { task_id: task.task_id.clone() }, text: "Keep the API stable".into(), thread_id: None,
    };
    let first = fixture.operator(action.clone());
    assert!(matches!(first.result, OrchestrationActionResult::DirectiveAdded { ref delivered_run_ids, duplicate: false, .. } if delivered_run_ids == &vec![worker.clone()]));
    let replay = fixture.operator(action);
    assert!(matches!(replay.result, OrchestrationActionResult::DirectiveAdded { duplicate: true, .. }));
    let state = fixture.state();
    assert_eq!(super::super::directives::active_for_run(&state, state.runs.iter().find(|r| r.run_id == worker).unwrap()).len(), 1);
    assert!(super::super::directives::active_for_run(&state, state.runs.iter().find(|r| r.run_id == sibling).unwrap()).is_empty());
    assert_eq!(state.messages.iter().filter(|m| m.kind == MessageKind::UserDirective && m.to_run_id == worker).count(), 1);
    fixture.operator(OrchestrationAction::DirectiveRemove { root_id: root.clone(), directive_id: directive_id.clone() });
    fixture.operator(OrchestrationAction::DirectiveRemove { root_id: root.clone(), directive_id });
    assert!(super::super::directives::active_for_root(&fixture.state(), &root).is_empty());
}

#[test]
fn worker_handoff_persists_and_does_not_wake_root() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "First");
    let worker = fixture.start(&root, &task);
    let message_id = id();
    let result = fixture.operator(OrchestrationAction::ThreadHandoff {
        message_id: message_id.clone(), thread_id: id(), to_run_id: root.clone(), text: "Please use this approach".into(),
        recipient: HandoffRecipient::TaskWorker, images: Vec::new(), task_id: Some(task.task_id.clone()),
        expected_task_revision: Some(task.task_revision.clone()), binding: None,
    });
    assert!(matches!(result.result, OrchestrationActionResult::Message { ref to_run_id, .. } if to_run_id == &worker));
    let state = fixture.state();
    let directive = state.directives.iter().find(|d| d.message_id.as_deref() == Some(&message_id)).unwrap();
    assert!(matches!(&directive.scope, DirectiveScope::Task { task_id } if task_id == &task.task_id));
    let copy = state.messages.iter().find(|m| m.message_id == format!("directive-copy:{}", directive.directive_id)).unwrap();
    assert!(!super::super::messages::inbox_meaningful(copy));
    let counts = super::super::messages::inbox_counts(std::slice::from_ref(copy), &root, 0);
    assert!(!counts.pending);
    assert!(counts.counts.is_empty());
}

#[test]
fn worker_handoff_rejects_stale_revision_and_missing_worker() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "First");
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let make_action = |revision: &str| OrchestrationAction::ThreadHandoff {
        message_id: id(), thread_id: id(), to_run_id: root.clone(), text: "Follow this".into(),
        recipient: HandoffRecipient::TaskWorker, images: Vec::new(), task_id: Some(task.task_id.clone()),
        expected_task_revision: Some(revision.into()), binding: None,
    };
    assert_code(fixture.apply(&actor, make_action("stale")), "task_revision_conflict");
    assert_code(fixture.apply(&actor, make_action(&task.task_revision)), "task_worker_unavailable");
    let worker = fixture.start(&root, &task);
    fixture.seed_run(&worker, |run| run.stage = RunStage::Closed);
    assert_code(fixture.apply(&actor, make_action(&task.task_revision)), "task_worker_unavailable");
}

#[test]
fn directive_authority_is_root_scoped() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Authority");
    let worker = fixture.start(&root, &task);
    let worker_actor = fixture.launch_and_bind(&worker);
    let root_actor = fixture.root_actor(&root);
    let other_root = fixture.root();
    let other_actor = fixture.root_actor(&other_root);
    let action = || OrchestrationAction::DirectiveAdd {
        directive_id: id(), root_id: root.clone(),
        scope: DirectiveScope::Task { task_id: task.task_id.clone() },
        text: "User instruction".into(), thread_id: None,
    };
    assert!(fixture.apply(&root_actor, action()).is_ok());
    assert_code(fixture.apply(&worker_actor, action()), "actor_forbidden");
    assert_code(fixture.apply(&other_actor, action()), "actor_forbidden");
    assert!(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), action()).is_ok());
}

#[test]
fn standing_directives_follow_project_scope() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let first = fixture.create_task(&root, "First project");
    let second = fixture.create_task(&root, "Second project");
    let worker = fixture.start(&root, &first);
    let sibling = fixture.start(&root, &second);
    let project = fixture.project();
    let mut other_project = project.clone();
    other_project.key = "another-project".into();
    fixture.seed_run(&worker, |run| run.project = Some(project.clone()));
    fixture.seed_run(&sibling, |run| run.project = Some(other_project.clone()));
    let scoped_id = id();
    let scoped = fixture.operator(OrchestrationAction::DirectiveAdd {
        directive_id: scoped_id.clone(), root_id: root.clone(),
        scope: DirectiveScope::Standing { project_key: Some(project.key.clone()) },
        text: "Project policy".into(), thread_id: None,
    });
    assert!(matches!(scoped.result, OrchestrationActionResult::DirectiveAdded { ref delivered_run_ids, .. } if delivered_run_ids == &vec![worker.clone()]));
    let global_id = id();
    fixture.operator(OrchestrationAction::DirectiveAdd {
        directive_id: global_id.clone(), root_id: root.clone(),
        scope: DirectiveScope::Standing { project_key: None },
        text: "Global policy".into(), thread_id: None,
    });
    let state = fixture.state();
    let worker_run = state.runs.iter().find(|run| run.run_id == worker).unwrap();
    let sibling_run = state.runs.iter().find(|run| run.run_id == sibling).unwrap();
    let worker_ids: Vec<_> = super::super::directives::active_for_run(&state, worker_run)
        .into_iter().map(|directive| directive.directive_id.as_str()).collect();
    let sibling_ids: Vec<_> = super::super::directives::active_for_run(&state, sibling_run)
        .into_iter().map(|directive| directive.directive_id.as_str()).collect();
    assert!(worker_ids.contains(&scoped_id.as_str()) && worker_ids.contains(&global_id.as_str()));
    assert_eq!(sibling_ids, vec![global_id.as_str()]);
}

#[test]
fn worker_directive_reads_hide_sibling_and_other_project() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let own_task = fixture.create_task(&root, "Own task");
    let sibling_task = fixture.create_task(&root, "Sibling task");
    let worker = fixture.start(&root, &own_task);
    let project = fixture.project();
    fixture.seed_run(&worker, |run| run.project = Some(project.clone()));
    let actor = fixture.launch_and_bind(&worker);
    let make = |scope| {
        let directive_id = id();
        fixture.operator(OrchestrationAction::DirectiveAdd {
            directive_id: directive_id.clone(), root_id: root.clone(), scope,
            text: "Instruction".into(), thread_id: None,
        });
        directive_id
    };
    let own = make(DirectiveScope::Task { task_id: own_task.task_id.clone() });
    let sibling = make(DirectiveScope::Task { task_id: sibling_task.task_id.clone() });
    let matching = make(DirectiveScope::Standing { project_key: Some(project.key.clone()) });
    let other = make(DirectiveScope::Standing { project_key: Some("other-project".into()) });
    let global = make(DirectiveScope::Standing { project_key: None });
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let direct = fixture.service.job_directives(caller).unwrap();
    let snapshot = fixture.service.job_snapshot_saved(caller, SESSION).unwrap();
    for directives in [&direct, &snapshot.directives] {
        let ids: Vec<_> = directives.iter().map(|directive| directive.directive_id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&own.as_str()));
        assert!(ids.contains(&matching.as_str()));
        assert!(ids.contains(&global.as_str()));
        assert!(!ids.contains(&sibling.as_str()));
        assert!(!ids.contains(&other.as_str()));
    }
}
