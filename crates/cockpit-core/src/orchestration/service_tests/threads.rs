use super::*;
use super::super::store::ThreadLinkRecord;

fn handoff(root: &str, thread: &str) -> OrchestrationAction {
    OrchestrationAction::ThreadHandoff {
        images: Vec::new(),
        message_id: id(), thread_id: thread.into(), to_run_id: root.into(),
        text: "Please investigate".into(), task_id: None,
        expected_task_revision: None, binding: None,
    }
}
#[test]
fn rich_thread_reports_survive_storage_and_inbox_projection() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.root_actor(&root);
    let question_thread = id();
    let result_thread = id();
    fixture.operator(handoff(&root, &question_thread));
    fixture.operator(handoff(&root, &result_thread));
    let question = "## Decision\n\nChoose **one**:\n- café\n- `code`\n\n<script>untrusted</script>";
    let result = "## Result\n\nFirst paragraph.\n\n- item one\n- item two\n\n```text\n  preserved\n```";
    for (kind, body, thread) in [(ReportKind::NeedsInput, question, &question_thread), (ReportKind::Result, result, &result_thread)] {
        let mut action = report(kind, (kind == ReportKind::Result).then_some(ReportOutcome::Succeeded), None);
        if let OrchestrationAction::Report { thread_id, summary, .. } = &mut action {
            *thread_id = Some(thread.clone());
            *summary = body.into();
        }
        fixture.apply(&actor, action).unwrap();
    }
    // Read back from the durable store, rather than projecting the action itself.
    let state = fixture.state();
    for body in [question, result] {
        let message = state.messages.iter().find(|m| m.text == body).unwrap();
        assert_eq!(message.report.as_ref().unwrap().summary, body);
    }
    let view = super::super::inbox_view::project(
        &InboxRequest { session_id: SESSION.into() }, &state, vec![],
        Err(InspectionError::new("offline", "unavailable")), "2026-10-05T12:00:00Z", "store",
    ).unwrap();
    let question_view = view.threads.iter().find(|t| t.thread_id == question_thread).unwrap();
    let result_view = view.threads.iter().find(|t| t.thread_id == result_thread).unwrap();
    assert_eq!(question_view.questions[0].text.text, question);
    assert_eq!(result_view.reply.as_ref().unwrap().text, result);
    assert_eq!(serde_json::to_value(question_view).unwrap()["questions"][0]["text"]["text"], question);
    assert_eq!(serde_json::to_value(result_view).unwrap()["reply"]["text"], result);
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

#[cfg(target_os = "linux")]
#[test]
fn new_handoffs_exclude_only_proven_dead_settled_owners() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let thread = id();
    let action = handoff(&root, &thread);
    fixture.operator(action.clone());
    let mut dead = live_process();
    dead.start_ticks = dead.start_ticks.checked_add(1).unwrap();
    let machine = crate::process_identity::machine_id().unwrap();
    fixture.seed_run(&root, |run| {
        run.stage = RunStage::Active;
        run.dispatch = None;
        let attachment = run.attachment.as_mut().unwrap();
        attachment.state = AttachmentState::Connected;
        attachment.process = Some(dead.clone());
        attachment.machine_id = Some(machine.clone());
    });
    let operator = Actor::Operator(OperatorOrigin::Browser);
    let before = fixture.state_bytes();
    assert_code(fixture.apply(&operator, handoff(&root, &id())), "thread_owner_invalid");
    assert_eq!(fixture.state_bytes(), before);
    fixture.operator(action); // Replay remains idempotent.
    fixture.operator(handoff(&root, &thread)); // Historical follow-up remains possible.
    let project = |state: &store::OrchestrationState| super::super::inbox_view::project(
        &InboxRequest { session_id: SESSION.into() }, state, vec![],
        Err(InspectionError::new("offline", "unavailable")), "2026-10-05T12:00:00Z", "store",
    ).unwrap().owners.into_iter().find(|owner| owner.root_id == root).unwrap().availability;
    assert_eq!(project(&fixture.state()), OwnerAvailability::Absent);

    fixture.seed_run(&root, |run| run.attachment.as_mut().unwrap().machine_id = None);
    assert_eq!(project(&fixture.state()), OwnerAvailability::Unknown);
    fixture.operator(handoff(&root, &id()));

    fixture.seed_run(&root, |run| {
        run.attachment.as_mut().unwrap().machine_id = Some(machine);
        run.stage = RunStage::Starting;
    });
    assert_eq!(project(&fixture.state()), OwnerAvailability::Starting);
    fixture.operator(handoff(&root, &id()));
    for phase in [ResumePhase::Reconciling, ResumePhase::Launching] {
        fixture.seed_run(&root, |run| {
            run.stage = RunStage::Active;
            run.resume = Some(ResumeRecord {
                operation_id: id(), run_id: run.run_id.clone(), previous_attachment_epoch: Some(1),
                fresh_conversation: None, previous_conversation: None, session_namespace: None,
                phase, cause: None, decision_id: None, replacement_attachment_epoch: None, updated_at: now(),
            });
        });
        assert_eq!(project(&fixture.state()), OwnerAvailability::Unknown);
        fixture.operator(handoff(&root, &id()));
    }
    fixture.seed_run(&root, |run| {
        run.resume = None;
        run.dispatch = Some(super::super::dispatch(DispatchStep::LaunchUnknown));
    });
    assert_eq!(project(&fixture.state()), OwnerAvailability::Unknown);
    fixture.operator(handoff(&root, &id()));
}

#[test]
fn missing_pane_does_not_prove_unknown_owner_dead() {
    let fixture = Fixture::new();
    let root = fixture.root();
    fixture.seed_run(&root, |run| {
        run.stage = RunStage::Active;
        run.dispatch = None;
        let attachment = run.attachment.as_mut().unwrap();
        attachment.process = None;
        attachment.location = Some(RunLocation {
            boot_id: None, peer_process: None, terminal_id: Some("terminal".into()),
            native_session_id: None, endpoint_identity: "endpoint".into(), session_id: SESSION.into(),
            workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(),
            launch_tag: "tag".into(),
        });
    });
    let view = super::super::inbox_view::project(
        &InboxRequest { session_id: SESSION.into() }, &fixture.state(), vec![],
        Ok(super::super::herdr::RuntimeView {
            endpoint_identity: "endpoint".into(), boot_id: None, peer_process: None,
            focused_workspace_id: None, workspaces: vec![], panes: vec![],
        }), "2026-10-05T12:00:00Z", "store",
    ).unwrap();
    assert_eq!(view.owners[0].availability, OwnerAvailability::Unknown);
    fixture.operator(handoff(&root, &id()));
}

#[test]
fn image_handoff_persists_metadata_and_fences_replay() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let operator = Actor::Operator(OperatorOrigin::Browser);
    let mut action = handoff(&root, &id());
    let OrchestrationAction::ThreadHandoff { text, images, .. } = &mut action else { unreachable!() };
    text.clear();
    *images = vec!["one".into(), "two".into()];
    let metadata = |id: &str| MessageImage {
        image_id: id.into(), mime_type: "image/png".into(), width: 1, height: 1, bytes: 1,
    };
    let resolved = ResolvedMutationInput { images: Some(vec![metadata("one"), metadata("two")]), ..Default::default() };
    let request = |action| OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None, action,
    };
    fixture.service.mutate_resolved(&operator, request(action.clone()), &resolved).unwrap();
    let state = fixture.state();
    assert_eq!(state.messages.last().unwrap().images, resolved.images.clone().unwrap());
    let before = fixture.state_bytes();
    let OrchestrationAction::ThreadHandoff { images, .. } = &mut action else { unreachable!() };
    images.reverse();
    let changed = ResolvedMutationInput { images: Some(vec![metadata("two"), metadata("one")]), ..Default::default() };
    assert_code(fixture.service.mutate_resolved(&operator, request(action), &changed), "operation_conflict");
    assert_eq!(fixture.state_bytes(), before);

    let mut empty = handoff(&root, &id());
    let OrchestrationAction::ThreadHandoff { text, .. } = &mut empty else { unreachable!() };
    text.clear();
    assert_code(fixture.apply(&operator, empty), "invalid_message");
    assert_eq!(fixture.state_bytes(), before);
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
