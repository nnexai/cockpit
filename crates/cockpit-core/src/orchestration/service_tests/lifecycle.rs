use super::*;

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
    assert_eq!(fixture.run(&root).stage, RunStage::Starting);
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
fn worker_start_replays_without_new_writer_and_fences_changed_payload() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Single start");
    let action = fixture.start_action(&root, &task);
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::WorkerStarted { start } = first.result else { unreachable!() };
    assert!(!start.duplicate);
    let run = fixture.run(&start.run_id);
    assert_eq!(run.task_revision_at_start.as_deref(), Some(task.task_revision.as_str()));
    assert_eq!(run.cleanup_policy, Some(WorkerCleanupPolicy::OwnedWhenSafe));
    assert_eq!(run.attachment.as_ref().unwrap().epoch, 1);
    let bytes = fixture.state_bytes();
    let replay = fixture.operator(action.clone());
    let OrchestrationActionResult::WorkerStarted { start: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.run_id, start.run_id);
    assert_eq!(fixture.state_bytes(), bytes);
    let mut changed = action;
    let OrchestrationAction::WorkerStart { brief, .. } = &mut changed else { unreachable!() };
    *brief = "A different operation payload".into();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        fixture.start_action(&root, &task)), "task_has_active_run");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn attachment_not_initialized_report_starts_work_and_plans_are_advisory() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "One start");
    let run_id = fixture.start(&root, &task);
    fixture.setup(&run_id);
    let actor = fixture.launch_and_bind(&run_id);
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    fixture.initialized(&actor);
    let run = fixture.run(&run_id);
    assert_eq!(run.stage, RunStage::Working);
    assert_eq!(run.last_report.as_ref().unwrap().kind, ReportKind::Initialized);
    assert_eq!(run.last_report.as_ref().unwrap().plan.as_deref(),
        Some("Implement the task, then report evidence"));
    fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Reported);
    assert!(!fixture.task(&root, &task.task_id).checked);
}


#[test]
fn offline_ordinary_updates_use_credentials_without_physical_provenance() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, mut actor) = fixture.working(&root);
    let before = fixture.run(&run_id);
    let caller = caller_mut(&mut actor);
    caller.provenance = None;
    caller.endpoint_identity = "unreachable-endpoint".into();
    caller.workspace_id.clear();
    caller.tab_id.clear();
    caller.pane_id.clear();
    caller.boot_id = None;
    caller.terminal_id = None;
    caller.native_session_id = None;
    caller.env_run = None;
    caller.omp_session_id = None;
    caller.main_omp_session_id = None;
    caller.agent_kind = None;
    caller.actual_agent_kind = None;
    caller.process = None;
    let edited = task_result(fixture.apply(&actor, OrchestrationAction::TaskUpdate {
        root_id: root.clone(), task_id: task.task_id.clone(),
        expected_task_revision: task.task_revision,
        title: None, description: Some("Offline durable job edit".into()),
    }).unwrap());
    assert_eq!(edited.description, "Offline durable job edit");
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    let after = fixture.run(&run_id);
    assert_eq!(after.attachment.as_ref().unwrap().epoch, before.attachment.as_ref().unwrap().epoch);
    assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
    assert_eq!(serde_json::to_value(after.attachment).unwrap(), serde_json::to_value(before.attachment).unwrap());
}

#[test]
fn conversation_rollover_does_not_replace_epoch_credentials_or_checkout_claim() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, run_id, mut actor) = fixture.working(&root);
    let before = fixture.run(&run_id);
    let caller = caller_mut(&mut actor);
    let credential_id = caller.auth.credential_id.clone();
    caller.omp_session_id = Some("rolled-over-main".into());
    caller.main_omp_session_id = caller.omp_session_id.clone();
    let action = OrchestrationAction::RunAttach {
        operation_id: id(),
        attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: "rolled-over-main".into(),
    };
    fixture.apply(&actor, action).unwrap();
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    let after = fixture.run(&run_id);
    let attachment = after.attachment.as_ref().unwrap();
    assert_eq!(attachment.epoch, before.attachment.as_ref().unwrap().epoch);
    assert_eq!(attachment.attachment_id, before.attachment.as_ref().unwrap().attachment_id);
    assert_eq!(attachment.main_session_id.as_deref(), Some("rolled-over-main"));
    assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
    assert!(fixture.state().credentials.iter().any(|record| record.credential_id == credential_id
        && record.state == credentials::CredentialState::Active));
}

#[test]
fn linked_answer_retries_preserve_committed_receipts_after_new_question_and_recheck_authority() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let (_, run_id, worker) = fixture.working(&root);
    fixture.apply(&worker, report(ReportKind::NeedsInput, None, None)).unwrap();
    let q1 = fixture.run(&run_id).last_report.unwrap().message_id;
    let answer = OrchestrationAction::MessageSend { message_id: id(),
    to_run_id: run_id.clone(),
    kind: MessageKind::Answer,
    text: "Use the approved choice".into(), in_reply_to: Some(q1.clone()), original_task_revision: None, original_claim_id: None };
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
        assert!(fixture.apply(&supervisor, changed).is_err());
        assert_eq!(fixture.state_bytes(), bytes);
    }
    let mut obsolete = supervisor.clone();
    caller_mut(&mut obsolete).auth.attachment_epoch += 1;
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&obsolete, answer), "job_unauthorized");
    assert_eq!(fixture.state_bytes(), bytes);
}

#[test]
fn bound_top_root_starts_answers_sends_back_accepts_and_cancels() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "Root-managed worker");
    let run_id = run_result(fixture.apply(&supervisor, fixture.start_action(&root, &task)).unwrap()).0;
    fixture.setup(&run_id);
    let worker = fixture.launch_and_bind(&run_id);
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    let native = fixture.run(&root).attachment.unwrap().main_session_id.unwrap();
    fixture
        .apply(&worker, report(ReportKind::NeedsInput, None, None))
        .unwrap();
    let question_id = fixture.run(&run_id).last_report.unwrap().message_id;
    let answer_id = id();
    fixture
        .apply(
            &supervisor,
            OrchestrationAction::MessageSend { message_id: answer_id.clone(),
            to_run_id: run_id.clone(),
            kind: MessageKind::Answer,
            text: "Use the established task scope; no additional decision is missing".into(), in_reply_to: Some(question_id.clone()), original_task_revision: None, original_claim_id: None },
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
    fixture.apply(&worker, fixture.successful_result(&worker))
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
    let question = projection::question_status(run, &inbox, &state.messages).unwrap();
    assert_eq!(question.question_message_id, sendback_question);
    assert!(matches!(question.receipt, QuestionReceipt::Unresolved));
    fixture.apply(&worker, fixture.successful_result(&worker))
        .unwrap();
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
    assert_code(fixture.apply(&supervisor, OrchestrationAction::Accept {
        run_id: run_id.clone(), expected_task_revision: edited.task_revision.clone(),
    }), "source_output_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.apply(&supervisor, OrchestrationAction::SendBack {
        run_id: run_id.clone(), text: "Report output for the updated canonical scope".into(),
    }).unwrap();
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    let result_id = fixture.run(&run_id).result.unwrap().message_id;
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
    let survivor_before = fixture.run(&survivor);
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
    assert_review_fields_only(&survivor_before, &fixture.run(&survivor));
    let cancel = fixture
        .state()
        .messages
        .into_iter()
        .find(|message| {
            message.to_run_id == cancelled && message.kind == MessageKind::CancelRequest
        })
        .unwrap();
    assert_agent_decision(&cancel, &root);
}

#[test]
fn management_requires_root_credentials_not_worker_child_or_forged_identity() {
    for operation in [0, 2, 3, 4, 5, 6] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let supervisor = fixture.launch_and_bind(&root);
        let task = fixture.create_task(&root, "Authority boundary");
        let run_id = fixture.start(&root, &task);
        fixture.setup(&run_id);
        let worker = fixture.launch_and_bind(&run_id);
        if matches!(operation, 2 | 3) {
            fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
        }
        let action = match operation {
            0 => fixture.start_action(&root, &fixture.create_task(&root, "Denied start")),
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
            5 => OrchestrationAction::MessageSend { message_id: id(),
            to_run_id: run_id.clone(),
            kind: MessageKind::Answer,
            text: "Answer".into(), in_reply_to: None, original_task_revision: None, original_claim_id: None },
            6 => OrchestrationAction::ReconcileRun {
                run_id: run_id.clone(),
                recovery: None,
            },
            _ => unreachable!(),
        };
        let other_root = fixture.root();
        let outsider = fixture.launch_and_bind(&other_root);
        let (_, branch_id, branch_actor) = fixture.working(&root);
        let mut inherited_env = branch_actor.clone();
        caller_mut(&mut inherited_env).env_run = Some((root.clone(), 1));
        let mut denied = vec![outsider, branch_actor.clone(), inherited_env];
        denied.push(worker);
        for fence in 0..3 {
            let mut actor = supervisor.clone();
            let caller = caller_mut(&mut actor);
            match fence {
                0 => caller.auth.credential_id = id(),
                1 => caller.auth.attachment_id = id(),
                2 => caller.auth.attachment_epoch += 1,
                _ => unreachable!(),
            }
            denied.push(actor);
        }
        let mut branch_subagent = fixture.child_actor(&branch_actor, &branch_id, "worker-child");
        // Physical claims of being main cannot enlarge this narrowed child credential.
        let caller = caller_mut(&mut branch_subagent);
        caller.agent_kind = Some(AgentKind::Main);
        caller.subagent_id = None;
        caller.env_run = Some((root.clone(), 1));
        caller.provenance = None;
        denied.push(branch_subagent);
        for actor in denied {
            let bytes = fixture.state_bytes();
            let failure = fixture.apply(&actor, action.clone()).unwrap_err();
            assert!(
                matches!(
                    failure.code.as_str(),
                    "actor_forbidden" | "caller_mismatch" | "session_mismatch" | "attempt_stale" | "job_unauthorized"
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
                    1 => run.stage = RunStage::Starting,
                    2 => run.stage = RunStage::Closed,
                    3 => run.parent_run_id = Some(branch_id.clone()),
                    4 => run.root_id = other_root.clone(),
                    5 => run.attachment.as_mut().unwrap().epoch += 1,
                    6 => run.attachment.as_mut().unwrap().state = AttachmentState::Revoked,
                    _ => unreachable!(),
                }
            });
            let bytes = fixture.state_bytes();
            let denied = fixture.apply(&supervisor, action.clone());
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
fn supervisor_recovers_only_strict_descendants() {
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
    let reviewed = fixture.run(&nested);
    let blocked = fixture.apply(&supervisor, fixture.resume_action(&nested)).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = blocked.result else { unreachable!() };
    assert_eq!(resume.resume.phase, ResumePhase::Blocked);
    assert_eq!(fixture.run(&nested).attachment.as_ref().unwrap().epoch,
        reviewed.attachment.as_ref().unwrap().epoch);
    for target in [&root, &other_root, &outsider, &branch] {
        let actor = if target == &branch { &branch_actor } else { &supervisor };
        for operation in 0..2 {
            let bytes = fixture.state_bytes();
            let result = if operation == 0 {
                fixture.apply(actor, OrchestrationAction::ReconcileRun { run_id: target.clone(), recovery: None })
            } else {
                fixture.apply(actor, fixture.resume_action(target))
            };
            if operation == 1 && (target == &root || target == &branch) {
                assert!(matches!(result.unwrap().result, OrchestrationActionResult::RunResumed { .. }));
            } else {
                assert_code(result, "actor_forbidden");
                assert_eq!(fixture.state_bytes(), bytes);
            }
        }
    }
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&supervisor, OrchestrationAction::ReconcileRun {
        run_id: nested.clone(), recovery: Some(cockpit_protocol::projects::WorkspaceRecoveryAction::RetryEnvironment),
    }), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.resume_exited(&supervisor, &nested);
    let retried = fixture.run(&nested);
    assert_eq!(retried.dispatch.as_ref().unwrap().launch_attempt, reviewed.dispatch.as_ref().unwrap().launch_attempt + 1);
    assert_eq!(retried.stage, RunStage::Starting);
    assert!(retried.attachment.as_ref().unwrap().location.is_none());
    assert_eq!(retried.task_id, reviewed.task_id);
    assert_eq!(retried.root_id, reviewed.root_id);
    assert_eq!(retried.brief, reviewed.brief);
    assert_eq!(serde_json::to_value(&retried.setup).unwrap(), serde_json::to_value(&reviewed.setup).unwrap());
    assert_eq!(serde_json::to_value(&retried.result).unwrap(), serde_json::to_value(&reviewed.result).unwrap());
    assert_eq!(fixture.state().operations.last().unwrap().actor_run_id.as_deref(), Some(root.as_str()));
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
            OrchestrationAction::MessageSend { message_id: id(),
            to_run_id: nested,
            kind: MessageKind::Answer,
            text: "Too late".into(), in_reply_to: None, original_task_revision: None, original_claim_id: None },
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
    caller.process = Some(live_process());
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let bootstrap = BootstrapCaller {
        endpoint_identity: caller.endpoint_identity.clone(), session_id: caller.session_id.clone(),
        workspace_id: caller.workspace_id.clone(), tab_id: caller.tab_id.clone(), pane_id: caller.pane_id.clone(),
        boot_id: caller.boot_id.clone(), terminal_id: caller.terminal_id.clone(),
        native_session_id: caller.native_session_id.clone(), omp_session_id: caller.omp_session_id.clone(),
        agent_kind: caller.agent_kind, actual_agent_kind: caller.actual_agent_kind.clone(), process: caller.process.clone(),
    };
    let mut shell = bootstrap.clone();
    shell.actual_agent_kind = Some(NativeAgentKind::Other("shell".into()));
    let bytes = fixture.state_bytes();
    assert!(fixture.apply(
        &Actor::Bootstrap(shell),
        OrchestrationAction::RunAdopt { label: "Not OMP".into() },
    ).is_err());
    assert_eq!(fixture.state_bytes(), bytes);
    let root = run_result(
        fixture
            .apply(
                &Actor::Bootstrap(bootstrap),
                OrchestrationAction::RunAdopt {
                    label: "Actual main explicitly adopted".into(),
                },
            )
            .unwrap(),
    )
    .0;
    assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().state, AttachmentState::Connecting);
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
    caller_mut(&mut actor).env_run = Some((root.clone(), 1));
    caller_mut(&mut actor).auth = fixture.auth(&root, JobRole::Root, None);
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root),
        "Adoption and staged credentials alone do not prove a current loaded extension");
    let Actor::Agent(caller) = &actor else { unreachable!() };
    fixture.apply(&actor, OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    }).unwrap();
    assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().state, AttachmentState::Connected);
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
    assert_eq!(fixture.run(&root).kind, RunKind::Adopted);
    let task = fixture.create_task(&root, "Adopted root task");
    let run_id = run_result(fixture.apply(&actor, fixture.start_action(&root, &task)).unwrap()).0;
    assert_eq!(fixture.run(&run_id).parent_run_id.as_deref(), Some(root.as_str()));
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
fn closing_root_retains_live_descendants_and_closed_root_evidence_inbox() {
    let fixture = Fixture::new();
    let root = fixture.root();
    fixture.launch_and_bind(&root);
    let (task, child, worker) = fixture.working(&root);
    let (_, nested, _) = fixture.working(&child);
    let child_before = fixture.run(&child);
    let nested_before = fixture.run(&nested);
    fixture.operator(OrchestrationAction::CancelRun {
        run_id: root.clone(),
    });
    assert_eq!(fixture.run(&root).stage, RunStage::Closed);
    assert_review_fields_only(&child_before, &fixture.run(&child));
    assert_review_fields_only(&nested_before, &fixture.run(&nested));
    assert!(!fixture.task(&root, &task.task_id).checked);
    let child_run = fixture.run(&child);
    let location = child_run.attachment.as_ref().unwrap().location.as_ref().unwrap();
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
        "fixture-store",
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
    fixture.apply(&worker, fixture.successful_result(&worker))
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
fn supervisor_worker_start_uses_canonical_fences_and_replays_before_machine_cas() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "CAS worker start");
    let action = fixture.start_action(&root, &task);
    let reviewed_revision = fixture.state().revision;
    fixture.operator(OrchestrationAction::Annotate {
        run_id: root.clone(), text: "Concurrent unrelated machine history".into(),
    });
    let request = OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: Some(reviewed_revision), action,
    };
    let first = fixture.service.mutate(&supervisor, request.clone()).unwrap();
    let OrchestrationActionResult::WorkerStarted { start } = first.result else { unreachable!() };
    assert!(!start.duplicate);
    assert_eq!(fixture.run(&start.run_id).parent_run_id.as_deref(), Some(root.as_str()));
    let bytes = fixture.state_bytes();
    let replay = fixture.service.mutate(&supervisor, request).unwrap();
    let OrchestrationActionResult::WorkerStarted { start: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.run_id, start.run_id);
    assert_eq!(fixture.state_bytes(), bytes);
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

