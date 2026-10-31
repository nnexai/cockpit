use super::*;
use super::super::store::ThreadLinkRecord;

fn handoff(root: &str, thread: &str) -> OrchestrationAction {
    OrchestrationAction::ThreadHandoff {
        message_id: id(), thread_id: thread.into(), to_run_id: root.into(),
        text: "Please investigate".into(), task_id: None,
        expected_task_revision: None, binding: None,
    }
}

#[test]
fn handoff_replay_survives_owner_close_and_rejects_changed_payload() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let thread = id();
    let action = handoff(&root, &thread);
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::Message { seq, .. } = first.result else {
        panic!("expected handoff message receipt");
    };
    fixture.seed_run(&root, |run| run.stage = RunStage::Closed);
    let before = serde_json::to_value(fixture.state().messages).unwrap();
    let replay = fixture.operator(action.clone());
    assert!(matches!(replay.result, OrchestrationActionResult::Message { seq: replay_seq, .. } if replay_seq == seq));
    assert_eq!(serde_json::to_value(fixture.state().messages).unwrap(), before);
    let mut changed = action;
    if let OrchestrationAction::ThreadHandoff { text, .. } = &mut changed {
        *text = "Different request".into();
    }
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
    assert_eq!(serde_json::to_value(fixture.state().messages).unwrap(), before);
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), handoff(&root, &id())), "thread_owner_invalid");
}

#[test]
fn replacement_preserves_explicit_handoff_placement() {
    for require_branch in [true, false] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Bound replacement");
        let worker = fixture.start(&root, &task);
        let project = ProjectIdentity {
            key: "git".into(), source: ProjectSource::Git { common_dir: "/repo/.git".into() },
            repository_id: None, source_workspace_id: Some("source-space".into()),
        };
        fixture.seed_run(&worker, |run| run.project = Some(project.clone()));
        let old = fixture.run(&worker);
        let commit = "a".repeat(40);
        let binding = HandoffBinding {
            repository_id: None, workspace_id: None,
            branch: require_branch.then(|| "feature".into()),
            base_ref: (!require_branch).then(|| commit.clone()),
        };
        let mut action = handoff(&root, &id());
        if let OrchestrationAction::ThreadHandoff { task_id, expected_task_revision, binding: requested, .. } = &mut action {
            *task_id = Some(task.task_id.clone());
            *expected_task_revision = Some(task.task_revision.clone());
            *requested = Some(binding.clone());
        }
        let operator = Actor::Operator(OperatorOrigin::Browser);
        fixture.service.mutate_resolved(&operator, OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: None, action,
        }, &ResolvedMutationInput { handoff_binding: Some(HandoffBindingRecord {
            requested: binding, repository_common_dir: None, workspace_checkout: None,
        }), ..Default::default() }).unwrap();
        let resolved = ResolvedMutationInput {
            project: Some(project), checkout: old.checkout.clone(),
            replacement_source: Some(ReplacementSource::GitHead {
                commit: commit.clone(), source_workspace_id: Some("source-space".into()),
            }),
            ..Default::default()
        };
        let action = OrchestrationAction::WorkerReplace {
            operation_id: id(), root_id: root.clone(), task_id: task.task_id.clone(),
            expected_run_id: worker.clone(), expected_assignment_generation: old.assignment_generation.unwrap(),
            reason: "Continue from the checkpoint".into(),
        };
        let before = fixture.state_bytes();
        let result = fixture.service.mutate_resolved(&operator, OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: None, action,
        }, &resolved);
        if require_branch {
            assert_code(result, "handoff_binding_mismatch");
            assert_eq!(fixture.state_bytes(), before, "rejected placement must not supersede the old attempt");
            assert_eq!(fixture.run(&worker).stage, old.stage);
        } else {
            let OrchestrationActionResult::WorkerReplaced { start, .. } = result.unwrap().result else { unreachable!() };
            assert_eq!(fixture.run(&worker).close_reason, Some(CloseReason::Superseded));
            assert!(matches!(fixture.run(&start.run_id).target,
                Some(DispatchTarget::SpaceWorktree { base_ref: Some(base), .. }) if base == commit));
        }
    }
}

#[test]
fn handoff_rejects_worker_and_fences_task_revision() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, worker, _) = fixture.working(&root);
    let operator = Actor::Operator(OperatorOrigin::Browser);
    let before = fixture.state_bytes();
    assert_code(fixture.apply(&operator, handoff(&worker, &id())), "thread_owner_invalid");
    assert_eq!(fixture.state_bytes(), before);
    let thread = id();
    let mut action = handoff(&root, &thread);
    if let OrchestrationAction::ThreadHandoff { task_id, .. } = &mut action {
        *task_id = Some(task.task_id.clone());
    }
    assert_code(fixture.apply(&operator, action.clone()), "task_revision_conflict");
    if let OrchestrationAction::ThreadHandoff { expected_task_revision, .. } = &mut action {
        *expected_task_revision = Some("obsolete".into());
    }
    assert_code(fixture.apply(&operator, action.clone()), "task_revision_conflict");
    assert_eq!(fixture.state_bytes(), before);
    if let OrchestrationAction::ThreadHandoff { expected_task_revision, .. } = &mut action {
        *expected_task_revision = Some(task.task_revision.clone());
    }
    fixture.apply(&operator, action).unwrap();
    let state = fixture.state();
    assert!(state.thread_links.iter().any(|link| link.thread_id == thread && link.root_id == root && link.task_id == task.task_id));
    let message = state.messages.iter().find(|message| message.thread_id.as_deref() == Some(thread.as_str())).unwrap();
    let record = message.handoff.as_ref().unwrap();
    assert_eq!(record.task_revision.as_deref(), Some(task.task_revision.as_str()));
    assert_eq!(record.attempt_run_id.as_deref(), Some(worker.as_str()));
}

#[test]
fn links_cannot_cross_roots_even_when_both_received_handoffs() {
    let fixture = Fixture::new();
    let first = fixture.root();
    let second = fixture.root();
    let first_task = fixture.create_task(&first, "First task");
    let second_task = fixture.create_task(&second, "Second task");
    let thread = id();
    fixture.operator(handoff(&first, &thread));
    fixture.operator(handoff(&second, &thread));
    let link = |root: &str, task: &Task| OrchestrationAction::ThreadLink {
        root_id: root.into(), task_id: task.task_id.clone(), thread_id: thread.clone(),
    };
    fixture.operator(link(&first, &first_task));
    let before = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), link(&second, &second_task)), "thread_root_conflict");
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn read_marks_require_operator_and_root_result_in_the_same_thread() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.root_actor(&root);
    let thread = id();
    let handoff = handoff(&root, &thread);
    let message_id = match &handoff { OrchestrationAction::ThreadHandoff { message_id, .. } => message_id.clone(), _ => unreachable!() };
    fixture.operator(handoff);
    let mark = |thread_id: &str, through_message_id: &str| OrchestrationAction::ThreadMarkRead {
        thread_id: thread_id.into(), through_message_id: through_message_id.into(),
    };
    let operator = Actor::Operator(OperatorOrigin::Browser);
    assert_code(fixture.apply(&operator, mark(&thread, &message_id)), "thread_result_invalid");
    let mut result = report(ReportKind::Result, Some(ReportOutcome::Succeeded), None);
    let result_id = if let OrchestrationAction::Report { message_id, thread_id, .. } = &mut result {
        *thread_id = Some(thread.clone());
        message_id.clone()
    } else { unreachable!() };
    fixture.apply(&actor, result).unwrap();
    assert_code(fixture.apply(&operator, mark(&id(), &result_id)), "thread_result_invalid");
    assert!(fixture.apply(&actor, mark(&thread, &result_id)).is_err());
    assert!(fixture.state().thread_reads.is_empty());
    fixture.apply(&operator, mark(&thread, &result_id)).unwrap();
    fixture.apply(&operator, mark(&thread, &result_id)).unwrap();
    let state = fixture.state();
    assert_eq!(state.thread_reads.len(), 1);
    assert_eq!(state.thread_reads[0].through_message_id, result_id);
}

#[test]
fn task_create_recovers_prejournaled_link_and_replays_without_duplicate() {
    let fixture = Fixture::new();
    let root = fixture.root();
    fixture.root_actor(&root);
    let thread = id();
    fixture.operator(handoff(&root, &thread));
    let task_id = id();
    // The durable link can precede tasks.md when a process stops between writes.
    fixture.seed_state(|state| state.thread_links.push(ThreadLinkRecord {
        thread_id: thread.clone(), root_id: root.clone(), task_id: task_id.clone(),
        linked_by: ActorRef::Operator, at: now(),
    }));
    let action = OrchestrationAction::TaskCreate {
        root_id: root.clone(), task_id: task_id.clone(), title: "Journal recovery".into(),
        description: "Resume the interrupted creation".into(), depends_on: Vec::new(),
        follow_up_of: None, expected_doc_revision: None, source_revision: None,
        thread_id: Some(thread.clone()),
    };
    let created = task_result(fixture.operator(action.clone()));
    let replay = task_result(fixture.operator(action));
    assert_eq!(created.task_id, task_id);
    assert_eq!(serde_json::to_value(created).unwrap(), serde_json::to_value(replay).unwrap());
    assert_eq!(fixture.state().thread_links.iter().filter(|link| link.thread_id == thread && link.root_id == root && link.task_id == task_id).count(), 1);
}

#[test]
fn reports_cannot_claim_another_roots_handoff_but_explicit_handoffs_authorize_each_owner() {
    let fixture = Fixture::new();
    let first = fixture.root();
    let second = fixture.root();
    let first_actor = fixture.root_actor(&first);
    let second_actor = fixture.root_actor(&second);
    let first_task = fixture.create_task(&first, "Owner task");
    let thread = id();
    fixture.operator(handoff(&first, &thread));
    // Merely mentioning the thread in another root's message is not a handoff.
    fixture.seed_state(|state| {
        let mut correlation = state.messages.iter().find(|message|
            message.thread_id.as_deref() == Some(thread.as_str())).unwrap().clone();
        correlation.message_id = id();
        correlation.from = ActorRef::Run { run_id: second.clone() };
        correlation.to_run_id = second.clone();
        correlation.handoff = None;
        state.messages.push(correlation);
    });
    let thread_report = |thread: &str| {
        let mut action = report(ReportKind::Progress, None, None);
        if let OrchestrationAction::Report { thread_id, .. } = &mut action {
            *thread_id = Some(thread.into());
        }
        action
    };
    let before = fixture.state_bytes();
    assert_code(fixture.apply(&second_actor, thread_report(&thread)), "thread_report_forbidden");
    assert_eq!(fixture.state_bytes(), before);
    fixture.apply(&first_actor, OrchestrationAction::ThreadLink {
        root_id: first.clone(), task_id: first_task.task_id.clone(), thread_id: thread.clone(),
    }).unwrap();
    fixture.apply(&first_actor, thread_report(&thread)).unwrap();
    fixture.apply(&second_actor, thread_report(&id())).unwrap();
    fixture.operator(handoff(&second, &thread));
    fixture.apply(&second_actor, thread_report(&thread)).unwrap();
    assert!(fixture.state().thread_links.iter().any(|link|
        link.thread_id == thread && link.root_id == first && link.task_id == first_task.task_id));
}

#[test]
fn report_thread_authority_never_crosses_sessions() {
    let fixture = Fixture::new();
    let first = fixture.root();
    let second = fixture.root();
    let second_actor = fixture.root_actor(&second);
    let thread = id();
    fixture.operator(handoff(&first, &thread));
    fixture.operator(handoff(&second, &thread));
    fixture.seed_run(&first, |run| run.session_id = "other-session".into());
    let mut action = report(ReportKind::Progress, None, None);
    if let OrchestrationAction::Report { thread_id, .. } = &mut action {
        *thread_id = Some(thread);
    }
    let before = fixture.state_bytes();
    assert_code(fixture.apply(&second_actor, action), "thread_report_forbidden");
    assert_eq!(fixture.state_bytes(), before);
}
