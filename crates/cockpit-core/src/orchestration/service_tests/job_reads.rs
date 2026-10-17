use super::*;

fn caller(actor: &Actor) -> &AgentCaller {
    let Actor::Agent(caller) = actor else { panic!("agent caller") };
    caller
}

fn private_message(state: &mut OrchestrationState, recipient: &str, text: &str) {
    messages::append(state, messages::AppendMessage {
        from: ActorRef::Operator, to_run_id: recipient, message_id: &id(),
        kind: MessageKind::Instruction, text, in_reply_to: None,
        report: None, stale: false, escalated_from: None,
    }).unwrap();
}

fn resource(run: &Run) -> OwnedResourceRecord {
    OwnedResourceRecord {
        resource_id: format!("resource-{}", run.run_id), root_id: run.root_id.clone(),
        run_id: run.run_id.clone(), kind: OwnedResourceKind::Space,
        ownership: ResourceOwnership::Owned, creation_operation_id: Some(id()),
        identity: OwnedResourceIdentity::Space { identity: OwnedSpaceIdentity {
            creation_operation_id: id(), endpoint_identity: "fixture".into(),
            session_id: run.session_id.clone(), workspace_id: "fixture".into(),
            checkout_path: "/disposable-space".into(),
        } },
        authority: CleanupAuthority::OwnedWhenSafe, consumers: Vec::new(), integration_result_id: None,
        cleanup: CleanupItem { operation_id: None, state: CleanupState::Running,
            reason: None, effects: Vec::new(), updated_at: now() },
    }
}

#[test]
fn credentialed_reads_isolate_every_projection_and_redact_own_and_sibling_launch_capabilities() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (own_task, own, worker_actor) = fixture.working(&root);
    let (_, sibling, _) = fixture.working(&root);
    let root_actor = fixture.root_actor(&root);
    let foreign_root = fixture.root();
    let (foreign_task, foreign, _) = fixture.working(&foreign_root);
    fixture.seed_state(|state| {
        for recipient in [&own, &sibling, &root, &foreign] {
            private_message(state, recipient, &format!("private-inbox-{recipient}"));
        }
        for run in &mut state.runs {
            if let Some(dispatch) = &mut run.dispatch {
                dispatch.latch = Some(AgentLaunchReceipt {
                    latch: format!("/private/latches/{}-SECRET-NONCE-MARKER.fifo", run.run_id),
                    nonce: "SECRET-NONCE-MARKER".into(), latch_device: "1".into(), latch_inode: "2".into(),
                });
            }
            if run.run_id == foreign {
                run.resume = Some(ResumeRecord {
                    operation_id: id(), run_id: foreign.clone(), previous_attachment_epoch: Some(1),
                    phase: ResumePhase::Connected, cause: None, decision_id: None,
                    replacement_attachment_epoch: None, updated_at: now(),
                });
            }
        }
        for (root_id, task, run_id) in [(&root, &own_task, &own), (&foreign_root, &foreign_task, &foreign)] {
            state.assignment_intents.push(store::AssignmentIntent {
                root_id: root_id.clone(), task_id: task.task_id.clone(), request_hash: id(),
                title: "Private assignment draft".into(), body: "Private draft".into(),
                origin: OperatorOrigin::Browser, state: IntentState::Conflict,
            });
            state.task_intents.push(TaskIntent {
                intent_id: id(), root_id: root_id.clone(), task_id: task.task_id.clone(),
                run_id: run_id.clone(), expected_task_revision: task.task_revision.clone(), state: IntentState::Conflict,
                origin: Some(DecisionOrigin::Browser), supervisor_run_id: None, conversation: None,
                result_message_id: None,
            });
            let run = state.runs.iter().find(|run| run.run_id == *run_id).unwrap();
            state.owned_resources.push(resource(run));
        }
        let mut closed = state.runs.iter().find(|run| run.run_id == foreign).unwrap().clone();
        closed.run_id = id();
        closed.stage = RunStage::Closed;
        state.owned_resources.push(resource(&closed));
        state.runs.push(closed);
    });
    let before = fixture.state_bytes();
    let task_path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let task_before = std::fs::read(&task_path).unwrap();
    let snapshot = fixture.service.job_snapshot_saved(caller(&worker_actor), SESSION).unwrap();
    let json = serde_json::to_string(&snapshot).unwrap();
    for secret in [&foreign_root, &foreign, &foreign_task.task_id] {
        assert!(!json.contains(secret), "foreign identity {secret}");
    }
    assert!(!json.contains("SECRET-NONCE-MARKER"));
    assert!(snapshot.runs.iter().any(|run| run.run_id == sibling));
    assert_eq!(snapshot.roots.len(), 1);
    assert_eq!(snapshot.board.as_ref().unwrap().root_id, root);
    assert!(snapshot.messages.iter().all(|message| message.to_run_id == own));
    assert!(json.contains(&format!("private-inbox-{own}")));
    assert!(!json.contains(&format!("private-inbox-{sibling}")));
    assert!(!json.contains(&format!("private-inbox-{root}")));
    assert_eq!(snapshot.assignment_intents.len(), 1);
    assert_eq!(snapshot.intents.len(), 1);
    assert_eq!(snapshot.restoration.total_attachments, 0);
    assert_eq!(snapshot.restoration.cleanup_pending, 0);
    assert!(snapshot.cleanup.iter().all(|summary| summary.run_id != foreign));
    for run in &snapshot.runs {
        if let Some(receipt) = run.dispatch.as_ref().and_then(|dispatch| dispatch.latch.as_ref()) {
            assert_eq!(receipt.latch, "[REDACTED]");
            assert_eq!(receipt.nonce, "[REDACTED]");
            assert_eq!((&*receipt.latch_device, &*receipt.latch_inode), ("1", "2"));
        }
    }
    let supervisor = fixture.service.job_snapshot_saved(caller(&root_actor), SESSION).unwrap();
    let supervisor_json = serde_json::to_string(&supervisor).unwrap();
    for recipient in [&own, &sibling, &root] {
        assert!(supervisor_json.contains(&format!("private-inbox-{recipient}")));
    }
    assert!(!supervisor_json.contains(&foreign_root));
    assert!(!supervisor_json.contains("SECRET-NONCE-MARKER"));
    let operator = fixture.service.snapshot_saved(&OrchestrationSnapshotRequest {
        session_id: SESSION.into(), root_id: Some(root.clone()),
    }).unwrap();
    let operator_json = serde_json::to_string(&operator).unwrap();
    assert!(operator_json.contains(&foreign_root));
    assert!(operator_json.contains("SECRET-NONCE-MARKER"));
    assert_eq!(operator.restoration.total_attachments, 1);
    assert_eq!(operator.restoration.cleanup_pending, 1);
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(std::fs::read(task_path).unwrap(), task_before);
}

#[test]
fn credentialed_reads_reject_stale_or_forged_identity_without_writes() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let original = caller(&actor);
    for fence in ["credential", "epoch", "generation", "conversation", "secret"] {
        let mut forged = original.clone();
        match fence {
            "credential" => forged.auth.credential_id = id(),
            "epoch" => forged.auth.attachment_epoch += 1,
            "generation" => forged.assignment_generation = Some(0),
            "conversation" => forged.conversation.as_mut().unwrap().session_id = "sibling-alias".into(),
            "secret" => forged.auth.secret = SecretBytes::new([42; 32]),
            _ => unreachable!(),
        }
        let before = fixture.state_bytes();
        assert_eq!(fixture.service.job_snapshot_saved(&forged, SESSION).unwrap_err().code, "job_unauthorized", "{fence}");
        assert_eq!(fixture.state_bytes(), before);
    }
    let before = fixture.state_bytes();
    assert_eq!(fixture.service.job_snapshot_saved(original, "foreign-session").unwrap_err().code, "session_mismatch");
    assert_eq!(fixture.state_bytes(), before);
    fixture.seed_state(|state| {
        let ownership = state.task_ownership.iter_mut().find(|record|
            record.owner.as_ref().is_some_and(|owner| owner.run_id == worker)).unwrap();
        ownership.generation += 1;
    });
    let before = fixture.state_bytes();
    assert_eq!(fixture.service.job_snapshot_saved(original, SESSION).unwrap_err().code, "job_unauthorized");
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn private_question_aliases_and_answers_are_hidden_but_public_run_reports_remain_visible() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, own, own_actor) = fixture.working(&root);
    let (_, sibling, sibling_actor) = fixture.working(&root);
    let (_, progress, progress_actor) = fixture.working(&root);
    let (_, result_run, result_actor) = fixture.working(&root);
    let root_actor = fixture.root_actor(&root);
    let mut questions = Vec::new();
    for (run_id, actor, marker) in [
        (&own, &own_actor, "OWN-QUESTION-BODY"),
        (&sibling, &sibling_actor, "SIBLING-PRIVATE-QUESTION-BODY"),
        (&root, &root_actor, "ROOT-PRIVATE-QUESTION-BODY"),
    ] {
        let message_id = id();
        fixture.apply(actor, OrchestrationAction::Report {
            message_id: message_id.clone(), kind: ReportKind::NeedsInput, outcome: None,
            summary: marker.into(), plan: Some(format!("{marker}-PLAN")),
            source_output: None, to_run_id: None,
            original_task_revision: None, original_claim_id: None,
        }).unwrap();
        questions.push((run_id.clone(), message_id));
    }
    for (run_id, question_id) in questions {
        fixture.operator(OrchestrationAction::MessageSend {
            message_id: id(), to_run_id: run_id.clone(), kind: MessageKind::Answer,
            text: format!("PRIVATE-ANSWER-BODY-{run_id}"), in_reply_to: Some(question_id),
            original_task_revision: None, original_claim_id: None,
        });
    }
    fixture.apply(&progress_actor, OrchestrationAction::Report {
        message_id: id(), kind: ReportKind::Progress, outcome: None,
        summary: "PUBLIC-PROGRESS-BODY".into(), plan: None,
        source_output: None, to_run_id: None,
        original_task_revision: None, original_claim_id: None,
    }).unwrap();
    let mut result = fixture.successful_result(&result_actor);
    let OrchestrationAction::Report { summary, .. } = &mut result else { unreachable!() };
    *summary = "PUBLIC-RESULT-BODY".into();
    fixture.apply(&result_actor, result).unwrap();
    let before = fixture.state_bytes();
    let worker = fixture.service.job_snapshot_saved(caller(&own_actor), SESSION).unwrap();
    let worker_json = serde_json::to_string(&worker).unwrap();
    for private in ["SIBLING-PRIVATE-QUESTION-BODY", "ROOT-PRIVATE-QUESTION-BODY"] {
        assert!(!worker_json.contains(private));
    }
    for run_id in [&sibling, &root] {
        assert!(!worker_json.contains(&format!("PRIVATE-ANSWER-BODY-{run_id}")));
        let report = worker.runs.iter().find(|run| run.run_id == *run_id).unwrap().last_report.as_ref().unwrap();
        assert_eq!(report.kind, ReportKind::NeedsInput);
        assert_eq!(report.summary, "[REDACTED]");
        assert!(report.plan.is_none());
    }
    assert!(worker_json.contains("OWN-QUESTION-BODY"));
    assert!(worker_json.contains("OWN-QUESTION-BODY-PLAN"));
    assert!(worker_json.contains(&format!("PRIVATE-ANSWER-BODY-{own}")));
    assert!(worker.questions.is_empty(), "questions derive only from the visible inbox events");
    assert!(!worker.attention.iter().any(|attention| attention.kind == AttentionKind::NeedsInput));
    assert_eq!(worker.runs.iter().find(|run| run.run_id == progress).unwrap()
        .last_report.as_ref().unwrap().summary, "PUBLIC-PROGRESS-BODY");
    let public_result = worker.runs.iter().find(|run| run.run_id == result_run).unwrap();
    assert_eq!(public_result.last_report.as_ref().unwrap().summary, "PUBLIC-RESULT-BODY");
    assert_eq!(public_result.result.as_ref().unwrap().summary, "PUBLIC-RESULT-BODY");
    let supervisor = fixture.service.job_snapshot_saved(caller(&root_actor), SESSION).unwrap();
    let supervisor_json = serde_json::to_string(&supervisor).unwrap();
    for private in ["SIBLING-PRIVATE-QUESTION-BODY", "ROOT-PRIVATE-QUESTION-BODY"] {
        assert!(supervisor_json.contains(private));
        assert!(supervisor_json.contains(&format!("{private}-PLAN")));
    }
    for run_id in [&own, &sibling, &root] {
        assert!(supervisor_json.contains(&format!("PRIVATE-ANSWER-BODY-{run_id}")));
    }
    assert!(supervisor.questions.iter().any(|question| question.run_id == sibling));
    assert_eq!(fixture.state_bytes(), before);
}
