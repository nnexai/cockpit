use super::*;

#[test]
fn successful_result_does_not_check_task_until_operator_accepts_exact_task_revision() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, actor) = fixture.working(&root);
    fixture.apply(&actor, fixture.successful_result(&actor))
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
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationAction::Accept {
            run_id: run_id.clone(), expected_task_revision: edited.task_revision.clone(),
        }), "source_output_conflict");
    assert_eq!(fixture.state_bytes(), bytes);
    fixture.operator(OrchestrationAction::SendBack {
        run_id: run_id.clone(), text: "Review the updated canonical criteria".into(),
    });
    fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
    assert_ne!(edited.task_revision, fixture.run(&run_id).task_revision_at_start.unwrap());
    fixture.operator(OrchestrationAction::Accept {
        run_id: run_id.clone(),
        expected_task_revision: edited.task_revision,
    });
    assert!(fixture.task(&root, &task.task_id).checked);
    let accepted = fixture.run(&run_id);
    assert_eq!(accepted.stage, RunStage::Closed);
    assert_eq!(accepted.close_reason, Some(CloseReason::Accepted));
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
fn invalid_credentials_never_publish_initialized_or_result_evidence() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run_id, actor) = fixture.working(&root);
    let foreign = fixture.root();
    let foreign_actor = fixture.launch_and_bind(&foreign);
    let Actor::Agent(foreign_caller) = foreign_actor else { unreachable!() };
    for fence in 0..4 {
        let mut denied = actor.clone();
        let caller = caller_mut(&mut denied);
        match fence {
            0 => caller.auth.credential_id = id(),
            1 => caller.auth.secret = foreign_caller.auth.secret.clone(),
            2 => caller.auth.attachment_id = id(),
            3 => caller.auth.attachment_epoch += 1,
            _ => unreachable!(),
        }
        for action in [
            report(ReportKind::Initialized, None, Some("Invalid diagnostic")),
            fixture.successful_result(&actor),
        ] {
            let before = fixture.state_bytes();
            assert!(fixture.apply(&denied, action).is_err());
            assert_eq!(fixture.state_bytes(), before);
        }
    }
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
    assert!(fixture.run(&run_id).result.is_none());
    assert!(!fixture.task(&root, &task.task_id).checked);
}


#[test]
fn acceptance_recovery_cannot_substitute_a_new_result_for_the_reviewed_receipt() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let (task, run_id, worker) = fixture.working(&root);
    fixture.apply(&worker, fixture.successful_result(&worker))
        .unwrap();
    let reviewed_result = fixture.run(&run_id).result.unwrap().message_id;
    let supervisor_native = fixture.run(&root).attachment.unwrap().conversation;
    let intent_id = fixture.pending_intent(&root, &task, &run_id);
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let intent = state
            .task_intents
            .iter_mut()
            .find(|intent| intent.intent_id == intent_id)
            .unwrap();
        intent.origin = Some(DecisionOrigin::Supervisor);
        intent.supervisor_run_id = Some(root.clone());
        intent.conversation = supervisor_native;
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
    fixture.apply(&parent_actor, fixture.successful_result(&parent_actor))
        .unwrap();
    let parent_before = fixture.run(&parent);
    let parent_result = serde_json::to_value(parent_before.result.as_ref().unwrap()).unwrap();
    let assert_parent_receipts = || {
        let after = fixture.run(&parent);
        assert_eq!(after.stage, RunStage::Reported);
        assert_eq!(
            serde_json::to_value(after.result.as_ref().unwrap()).unwrap(),
            parent_result
        );
        assert_eq!(
            serde_json::to_value(after.last_report.as_ref().unwrap()).unwrap(),
            parent_result
        );
    };
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
                    source_output: (kind == ReportKind::Result).then(|| SourceOutput::NonCode {
                        result_message_id: message_id.clone(),
                        task_revision: fixture.task(&root, fixture.run(&child).task_id.as_deref().unwrap()).task_revision,
                        artifacts: Vec::new(), focused_checks: Vec::new(),
                    }),
                    to_run_id: None,
                    original_task_revision: None,
                    original_claim_id: None,
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
        assert_eq!(delivered.stage, DeliveryStage::Stored);
        let receipt = delivered.report.as_ref().unwrap();
        assert_eq!(receipt.kind, kind);
        assert_eq!(receipt.summary, summary);
        assert_eq!(receipt.outcome, outcome);
        let child_after = fixture.run(&child);
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
fn nested_reports_and_question_pointers_wake_root_once_without_forwarding_subagent_evidence() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, branch, _) = fixture.working(&root);
    let task = fixture.create_task(&root, "Nested managed worker");
    let run_id = fixture.start(&branch, &task);
    fixture.setup(&run_id);
    let worker = fixture.launch_and_bind(&run_id);
    let initialized = report(ReportKind::Initialized, None, Some("Advisory work plan"));
    let needs_input = report(ReportKind::NeedsInput, None, None);
    let result = fixture.successful_result(&worker);
    for (event, action) in [
        ("initialized", initialized),
        ("needs_input", needs_input),
        ("result", result),
    ] {
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
            })
        );
        assert!(!pointers[0].text.contains("Evidence from isolated worker"));
    }
    let original = fixture.run(&run_id);
    let mut stale = worker;
    caller_mut(&mut stale).auth.attachment_epoch += 1;
    for actor in [&stale] {
        let action = report(ReportKind::NeedsInput, None, None);
        let OrchestrationAction::Report { message_id, .. } = &action else {
            unreachable!()
        };
        let pointer_id = format!("manage-{run_id}-{message_id}");
        assert_code(fixture.apply(actor, action), "job_unauthorized");
        assert!(
            fixture
                .state()
                .messages
                .iter()
                .all(|message| message.message_id != pointer_id)
        );
        assert_review_fields_only(&original, &fixture.run(&run_id));
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
fn acceptance_intent_recovery_cannot_accept_cancelled_restarted_failed_or_other_task_runs() {
    for race in 0..7 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (task, run_id, actor) = fixture.working(&root);
        fixture.apply(&actor, fixture.successful_result(&actor))
            .unwrap();
        let intent_id = fixture.pending_intent(&root, &task, &run_id);
        fixture.seed_run(&run_id, |run| match race {
            0 => {
                run.stage = RunStage::Closed;
                run.close_reason = Some(CloseReason::Cancelled);
            }
            1 => {
                run.stage = RunStage::Starting;
                run.attachment.as_mut().unwrap().conversation = None;
                run.dispatch.as_mut().unwrap().launch_attempt += 1;
            }
            2 => run.result.as_mut().unwrap().outcome = Some(ReportOutcome::Failed),
            3 => run.result = None,
            4 => run.task_id = Some(id()),
            5 => run.root_id = id(),
            6 => run.stage = RunStage::Working,
            _ => unreachable!(),
        });
        let before = fixture.run(&run_id);
        let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
        reopened.recover_intents().unwrap();
        assert_review_fields_only(&before, &fixture.run(&run_id));
        assert!(!fixture.task(&root, &task.task_id).checked);
        let state = fixture.state();
        assert_eq!(state.task_intents.len(), 1);
        assert_eq!(state.task_intents[0].intent_id, intent_id);
        assert_eq!(state.task_intents[0].state, IntentState::Conflict);
    }
}
