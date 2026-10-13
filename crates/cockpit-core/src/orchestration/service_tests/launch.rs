use super::*;

fn review_error() -> ErrorResponse {
    ErrorResponse {
        code: "launch_identity_conflict".into(),
        message: "Fresh runtime proof conflicts with the recorded launch".into(),
    }
}





fn advance_worker(fixture: &Fixture, _run_id: &str, actor: &Actor, stage: RunStage) {
    fixture.initialized(actor);
    if stage == RunStage::Reported {
        fixture.apply(actor, fixture.successful_result(actor))
            .unwrap();
    }
}

#[test]
fn registration_activates_neutral_root_without_runtime_or_launch_ack() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.neutral_actor(&root);
    assert!(fixture.run(&root).attachment.as_ref().unwrap().conversation.is_none());
    fixture.apply(&actor, OrchestrationAction::RunRegister {
        operation_id: caller_auth(&actor).credential_id.clone(),
    }).unwrap();
    let registered = fixture.run(&root);
    assert_eq!(registered.stage, RunStage::Active);
    assert!(registered.attachment.as_ref().unwrap().location.is_none());
    assert!(registered.attachment.as_ref().unwrap().process.is_none());
    assert_eq!(registered.launch_config.harness_kind, "service-test");
    assert_eq!(registered.launch_config.command, "test-agent");
    assert_eq!(fixture.state().messages.iter().filter(|message|
        message.to_run_id == root && message.kind == MessageKind::SupervisorBrief).count(), 1);
}

#[test]
fn first_launch_proof_preserves_fast_same_launch_progress_and_repeated_proof_is_no_write() {
    for stage in [
        RunStage::Working,
        RunStage::Reported,
        RunStage::Active,
    ] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let run_id = if stage == RunStage::Active {
            fixture.setup(&root);
            root
        } else {
            let task = fixture.create_task(&root, "Fast same-launch progress");
            let run_id = fixture.start(&root, &task);
            fixture.setup(&run_id);
            run_id
        };
        let actor = fixture.launch_tab(&run_id);
        fixture
            .service
            .record_launch_pending(&fixture.run(&run_id))
            .unwrap();
        fixture
            .apply(
                &actor,
                OrchestrationAction::RunRegister { operation_id: id() },
            )
            .unwrap();
        let reviewed = fixture.run(&run_id);
        // Model same-incarnation report progress while runtime proof is read.
        fixture.seed_run(&run_id, |run| {
            run.stage = if stage == RunStage::Active {
                RunStage::Active
            } else {
                RunStage::Working
            };
        });
        if stage != RunStage::Active {
            // The registered worker is the current assigned task owner; a
            // supervisor has no task source Result to report.
            advance_worker(&fixture, &run_id, &actor, stage);
        }
        let before = fixture.run(&run_id);
        assert!(!before.dispatch.as_ref().unwrap().agent_started);
        let original_messages = serde_json::to_value(fixture.state().messages).unwrap();
        let revision = fixture.state().revision;
        assert_eq!(
            fixture.service.record_launch_verified(&reviewed).unwrap(),
            revision + 1
        );
        let after = fixture.run(&run_id);
        assert_eq!(after.stage, stage);
        assert_eq!(after.task_id, before.task_id);
        assert_eq!(after.root_id, before.root_id);
        assert_eq!(after.attempt, before.attempt);
        assert_eq!(after.checkout_claim_id, before.checkout_claim_id);
        assert_eq!(serde_json::to_value(&after.setup).unwrap(), serde_json::to_value(&before.setup).unwrap());
        assert_eq!(serde_json::to_value(&after.result).unwrap(), serde_json::to_value(&before.result).unwrap());
        let before_attachment = before.attachment.as_ref().unwrap();
        let after_attachment = after.attachment.as_ref().unwrap();
        assert_eq!(after_attachment.attachment_id, before_attachment.attachment_id);
        assert_eq!(after_attachment.epoch, before_attachment.epoch);
        assert_eq!(after_attachment.process, before_attachment.process);
        assert_eq!(after_attachment.conversation, before_attachment.conversation);
        assert_eq!(after_attachment.state, AttachmentState::Connected);
        assert!(after.dispatch.as_ref().unwrap().agent_started);
        assert_eq!(
            after.dispatch.as_ref().unwrap().step,
            DispatchStep::Launched
        );
        let kind = if stage == RunStage::Active { MessageKind::SupervisorBrief } else { MessageKind::WorkBrief };
        assert_eq!(serde_json::to_value(fixture.state().messages).unwrap(), original_messages);
        assert_eq!(fixture.state().messages.iter().filter(|message|
            message.to_run_id == run_id && message.kind == kind).count(), 1);
        // Repeated physical proof never reissues or reinitializes launch work.
        let revision = fixture.state().revision;
        let bytes = fixture.state_bytes();
        let receiver = fixture.service.revision.subscribe();
        assert_eq!(
            fixture.service.record_launch_verified(&reviewed).unwrap(),
            revision
        );
        assert_eq!(fixture.state_bytes(), bytes);
        assert!(!receiver.has_changed().unwrap());
    }
}



fn assert_proof_discarded(fixture: &Fixture, reviewed: &Run) {
    let revision = fixture.state().revision;
    let bytes = fixture.state_bytes();
    let receiver = fixture.service.revision.subscribe();
    assert_eq!(
        fixture.service.record_launch_verified(reviewed).unwrap(),
        revision
    );
    assert_eq!(
        fixture.state_bytes(),
        bytes,
        "Obsolete proof must not write"
    );
    assert!(
        !receiver.has_changed().unwrap(),
        "Obsolete proof must not notify"
    );
}

#[test]
fn registration_promotes_root_and_worker_directly_without_herdr_or_omp() {
    for worker in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let run_id = if worker {
            let task = fixture.create_task(&root, "Neutral worker");
            fixture.start(&root, &task)
        } else {
            root
        };
        let actor = fixture.neutral_actor(&run_id);
        fixture.apply(&actor, OrchestrationAction::RunRegister {
            operation_id: caller_auth(&actor).credential_id.clone(),
        }).unwrap();
        let run = fixture.run(&run_id);
        assert_eq!(run.stage, if worker { RunStage::Working } else { RunStage::Active });
        assert!(run.attachment.as_ref().unwrap().location.is_none());
        assert!(run.attachment.as_ref().unwrap().process.is_none());
        assert_eq!(run.attachment.as_ref().unwrap().conversation.as_ref(),
            match &actor { Actor::Agent(caller) => Some(&caller.conversation), _ => unreachable!() });
        let kind = if worker { MessageKind::WorkBrief } else { MessageKind::SupervisorBrief };
        assert_eq!(fixture.state().messages.iter().filter(|message|
            message.to_run_id == run_id && message.kind == kind).count(), 1);
    }
}

#[test]
fn launch_proof_fences_target_task_attachment_and_every_physical_receipt_field() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Exact launch proof");
    let run_id = fixture.start(&root, &task);
    fixture.setup(&run_id);
    let actor = fixture.launch_tab(&run_id);
    fixture
        .service
        .record_launch_pending(&fixture.run(&run_id))
        .unwrap();
    fixture
        .apply(
            &actor,
            OrchestrationAction::RunRegister { operation_id: id() },
        )
        .unwrap();
    let reviewed = fixture.run(&run_id);
    for fence in 0..20 {
        let mut stale = reviewed.clone();
        match fence {
            0 => stale.root_id = id(),
            1 => stale.task_id = Some(id()),
            2 => stale.session_id = "different-session".into(),
            3 => stale.attempt += 1,
            4 => stale.dispatch.as_mut().unwrap().launch_attempt += 1,
            5 => stale.dispatch.as_mut().unwrap().launch_tag = Some("different-tag".into()),
            6 => {
                stale.dispatch.as_mut().unwrap().endpoint_identity =
                    Some("different-endpoint".into())
            }
            7 => stale.attachment.as_mut().unwrap().attachment_id = id(),
            8 => stale.attachment.as_mut().unwrap().epoch += 1,
            9 => stale.attachment.as_mut().unwrap().launch_attempt += 1,
            10 => stale.attachment.as_mut().unwrap().location = None,
            11 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().boot_id = Some("different-boot".into()),
            12 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().terminal_id = Some("different-terminal".into()),
            13 => {
                stale.attachment.as_mut().unwrap().location.as_mut().unwrap().native_session_id = Some("different-native".into())
            }
            14 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().endpoint_identity = "different-endpoint".into(),
            15 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().session_id = "different-session".into(),
            16 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().workspace_id = "different-space".into(),
            17 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().tab_id = "different-tab".into(),
            18 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().pane_id = "different-pane".into(),
            19 => stale.attachment.as_mut().unwrap().location.as_mut().unwrap().launch_tag = "different-tag".into(),
            _ => unreachable!(),
        }
        assert_proof_discarded(&fixture, &stale);
    }
    // Even matching captured/current corrupt receipts are not valid proof.
    for fence in 0..3 {
        fixture.seed_run(&run_id, |run| {
            *run = reviewed.clone();
            match fence {
                0 => run.attachment.as_mut().unwrap().location.as_mut().unwrap().launch_tag.clear(),
                1 => {
                    run.dispatch.as_mut().unwrap().endpoint_identity = Some("wrong-endpoint".into())
                }
                2 => run.attachment.as_mut().unwrap().location.as_mut().unwrap().session_id = "wrong-session".into(),
                _ => unreachable!(),
            }
        });
        assert_proof_discarded(&fixture, &fixture.run(&run_id));
    }
    fixture.seed_run(&run_id, |run| {
        *run = reviewed;
        // Herdr's optional native ID is not a required launch gate.
        run.attachment.as_mut().unwrap().location.as_mut().unwrap().native_session_id = None;
    });
    fixture
        .service
        .record_launch_verified(&fixture.run(&run_id))
        .unwrap();
    assert_eq!(fixture.run(&run_id).stage, RunStage::Working);
}

#[test]
fn late_initial_physical_proof_cannot_revive_cancel_retry_registration_or_move() {
    for race in 0..4 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let actor = fixture.launch_tab(&root);
        fixture
            .service
            .record_launch_pending(&fixture.run(&root))
            .unwrap();
        if race != 2 {
            fixture
                .apply(
                    &actor,
                    OrchestrationAction::RunRegister { operation_id: id() },
                )
                .unwrap();
        }
        let reviewed = fixture.run(&root);
        match race {
            0 => {
                fixture.operator(OrchestrationAction::CancelRun {
                    run_id: root.clone(),
                });
            }
            1 => {
                fixture
                    .service
                    .record_dispatch(
                        &root,
                        DispatchUpdate::Step {
                            step: DispatchStep::LaunchUnknown,
                            error: Some(review_error()),
                        },
                    )
                    .unwrap();
                fixture.operator(fixture.resume_action(&root));
                let retained = fixture.run(&root);
                assert_eq!(retained.resume.as_ref().unwrap().phase, ResumePhase::Blocked,
                    "An uncertain initial launch has no root-main recovery authority");
                assert_eq!(retained.dispatch.unwrap().launch_attempt,
                    reviewed.dispatch.as_ref().unwrap().launch_attempt);
            }
            2 => {
                fixture
                    .apply(
                        &actor,
                        OrchestrationAction::RunRegister { operation_id: id() },
                    )
                    .unwrap();
            }
            3 => {
                let mut moved = actor.clone();
                let moved_caller = caller_mut(&mut moved);
                moved_caller.workspace_id = "moved-space".into();
                moved_caller.tab_id = "moved-tab".into();
                moved_caller.pane_id = "moved-pane".into();
                fixture.attach_observed(&moved, &root);
            }
            _ => unreachable!(),
        }
        assert_proof_discarded(&fixture, &reviewed);
        assert_eq!(fixture.state().messages.iter().filter(|message|
            message.to_run_id == root && message.kind == MessageKind::SupervisorBrief).count(), 1);
    }
}

fn dispatch_events(fixture: &Fixture, root: &str, event: &str) -> Vec<Message> {
    fixture.state().messages.into_iter().filter(|message| {
        message.to_run_id == root && matches!(message.from, ActorRef::Dispatcher)
            && serde_json::from_str::<serde_json::Value>(&message.text)
                .is_ok_and(|body| body["event"] == event)
    }).collect()
}

#[test]
fn dispatch_failure_wakes_root_once_with_typed_next_steps() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "Recover planning");
    let worker = fixture.start(&root, &task);
    let failure = || DispatchUpdate::PlanFailed { error: ErrorResponse {
        code: "endpoint_unavailable".into(), message: "Herdr endpoint is unavailable".into(),
    }};
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    let events = dispatch_events(&fixture, &root, "dispatch_failure");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, MessageKind::Observation);
    assert_eq!(events[0].stage, DeliveryStage::Stored);
    let run = fixture.run(&worker);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&events[0].text).unwrap(), serde_json::json!({
        "event": "dispatch_failure", "run_id": worker, "task_id": task.task_id,
        "run_attempt": run.attempt, "launch_attempt": run.dispatch.as_ref().unwrap().launch_attempt,
        "stage": run.stage, "step": "plan_failed", "agent_started": false,
        "error": {"code": "endpoint_unavailable", "message": "Herdr endpoint is unavailable"},
        "automatic": "none", "effect": "none", "next": ["show", "reconcile"], "operator_reason": null,
    }));
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 1);
    fixture.apply(&supervisor, OrchestrationAction::InboxPull { after_seq: 0, limit: 100 }).unwrap();
    fixture.service.record_dispatch(&worker, failure()).unwrap();
    assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 2);
    fixture.service.record_dispatch(&root, failure()).unwrap();
    assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 2);
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    let reopened_state = {
        let locked = reopened.store.lock().unwrap();
        locked.read().unwrap()
    };
    assert_eq!(reopened_state.revision, fixture.state().revision);
    assert_review_fields_only(&fixture.run(&worker),
        &reopened_state.runs[run_index(&reopened_state, &worker).unwrap()]);
    assert_eq!(serde_json::to_value(reopened_state.messages).unwrap(),
        serde_json::to_value(fixture.state().messages).unwrap());
}

#[test]
fn launch_review_failure_classifies_effect_and_operator_cases() {
    for case in 0..7 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Classified recovery");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        if matches!(case, 0 | 1 | 5 | 6) {
            fixture.launch_and_bind(&worker);
        }
        fixture.seed_run(&worker, |run| {
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.step = DispatchStep::LaunchIntent;
            if matches!(case, 2 | 3) {
                let setup = run.setup.as_mut().unwrap();
                setup.operation_id = Some("uncertain-operation".into());
                setup.workspace_id = None;
            }
            if case == 1 { run.attachment.as_mut().unwrap().process = None; }
            if case == 5 { run.stage = RunStage::Reported; }
            if case == 6 { run.stage = RunStage::Closed; }
        });
        if case == 6 {
            let bytes = fixture.state_bytes();
            fixture.service.record_launch_review(&fixture.run(&worker), DispatchStep::NeedsReview, Some(review_error())).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
        } else if matches!(case, 2 | 3) {
            fixture.service.record_dispatch(&worker, DispatchUpdate::Step {
                step: DispatchStep::SetupUnknown,
                error: Some(ErrorResponse { code: if case == 2 { "workspace_conflict" } else { "outcome_unknown" }.into(),
                    message: "Checkout effect is uncertain".into() }),
            }).unwrap();
        } else {
            fixture.service.record_launch_review(&fixture.run(&worker), DispatchStep::NeedsReview,
                Some(ErrorResponse { code: "launch_identity_conflict".into(), message: "é".repeat(1000) })).unwrap();
        }
        let events = dispatch_events(&fixture, &root, "dispatch_failure");
        if case >= 5 {
            assert!(events.is_empty());
            continue;
        }
        assert_eq!(events.len(), 1);
        let body: serde_json::Value = serde_json::from_str(&events[0].text).unwrap();
        let effect = if case == 4 { "none" } else { "unknown" };
        assert_eq!(body["effect"], effect);
        assert_eq!(body["operator_reason"].is_string(), matches!(case, 1..=3));
        assert!(body["error"]["message"].as_str().unwrap().len() <= 1024);
    }
}

fn observed_root_pane(fixture: &Fixture, root: &str) -> herdr::RuntimePane {
    if fixture.run(root).attachment.as_ref().unwrap().location.is_none() {
        fixture.launch_and_bind(root);
    }
    let location = fixture.run(root).attachment.unwrap().location.unwrap();
    herdr::RuntimePane {
        workspace_id: location.workspace_id,
        workspace_label: "Project".into(),
        tab_id: location.tab_id,
        tab_label: "Supervisor".into(),
        pane_id: location.pane_id,
        terminal_id: location.terminal_id,
        native_session_id: location.native_session_id,
        native_session_file: None,
        agent_name: None,
        agent_kind: Some("service-test".into()),
        launch_pending: false,
        agent_status: Some("working".into()),
        state_changed_at: None,
    }
}

#[test]
fn unstarted_unknown_launch_escalates_after_settle_and_recovers_once() {
    for exhausted in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Automatic proof before escalation");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        fixture.service.record_dispatch(&worker, DispatchUpdate::LaunchIntent {
            launch_tag: format!("launch-{worker}-1"), launch_attempt: 1, endpoint_identity: "endpoint-one".into(),
        }).unwrap();
        fixture.service.record_launch_review(&fixture.run(&worker), DispatchStep::LaunchUnknown, Some(ErrorResponse {
            code: "agent_start_unobserved".into(), message: "Agent start was not observed before its deadline".into(),
        })).unwrap();
        assert!(dispatch_events(&fixture, &root, "dispatch_failure").is_empty());
        let runtime = herdr::RuntimeView {
            endpoint_identity: "endpoint-one".into(), boot_id: Some("boot-one".into()),
            workspaces: Vec::new(), panes: vec![observed_root_pane(&fixture, &root)],
        };
        let bytes = fixture.state_bytes();
        let receiver = fixture.service.subscribe();
        fixture.service.record_observation(&runtime, SESSION).unwrap();
        assert_eq!(fixture.state_bytes(), bytes);
        assert!(!receiver.has_changed().unwrap());
        if exhausted {
            fixture.seed_run(&worker, |run| {
                run.dispatch.as_mut().unwrap().updated_at =
                    (time::OffsetDateTime::now_utc() - time::Duration::milliseconds(DEFAULT_START_TIMEOUT_MS as i64 + 1000))
                        .format(&time::format_description::well_known::Rfc3339).unwrap();
            });
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            let events = dispatch_events(&fixture, &root, "dispatch_failure");
            assert_eq!(events.len(), 1);
            assert!(events[0].message_id.ends_with(":launch_unresolved"));
            let body: serde_json::Value = serde_json::from_str(&events[0].text).unwrap();
            assert_eq!(body["automatic"], "exhausted");
            assert_eq!(body["effect"], "unknown");
            let bytes = fixture.state_bytes();
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
            fixture.seed_run(&worker, |run| {
                run.dispatch.as_mut().unwrap().error = Some(ErrorResponse { code: "launch_outcome_unknown".into(), message: "Different evidence, same launch".into() });
            });
            let bytes = fixture.state_bytes();
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
            assert_eq!(dispatch_events(&fixture, &root, "dispatch_failure").len(), 1);
        }
        // A late actual-main bind and proof resolves this same launch incarnation.
        let actor = fixture.launch_tab(&worker);
        fixture.apply(&actor, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
        let reviewed = fixture.run(&worker);
        fixture.service.record_launch_verified(&reviewed).unwrap();
        let notices = dispatch_events(&fixture, &root, "dispatch_recovered");
        assert_eq!(notices.len(), usize::from(exhausted));
        let bytes = fixture.state_bytes();
        fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
        assert_eq!(fixture.state_bytes(), bytes);
        assert_eq!(dispatch_events(&fixture, &root, "dispatch_recovered").len(), usize::from(exhausted));
    }
}

#[test]
fn supervisor_recovery_preserves_machine_cas_and_automatic_launch_ownership() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let supervisor = fixture.launch_and_bind(&root);
    let task = fixture.create_task(&root, "Uncertain setup recovery");
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    fixture.service.record_dispatch(&worker, DispatchUpdate::Step {
        step: DispatchStep::SetupUnknown, error: Some(review_error()),
    }).unwrap();
    let revision = fixture.state().revision;
    fixture.operator(OrchestrationAction::Annotate { run_id: root.clone(), text: "Unrelated newer history".into() });
    for action in [
        OrchestrationAction::ReconcileRun { run_id: worker.clone(), recovery: None },
    ] {
        let request = OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: Some(revision), action };
        let bytes = fixture.state_bytes();
        let result = fixture.service.mutate(&supervisor, request);
        assert_code(result, "orchestration_revision_conflict");
        assert_eq!(fixture.state_bytes(), bytes);
    }
    fixture.seed_run(&worker, |run| {
        run.dispatch.as_mut().unwrap().step = DispatchStep::SetupUnknown;
        run.dispatch.as_mut().unwrap().launch_tag = None;
        run.setup.as_mut().unwrap().operation_id = Some("checkout-recovery-operation".into());
        run.setup.as_mut().unwrap().workspace_id = None;
    });
    fixture.apply(&supervisor, OrchestrationAction::ReconcileRun {
        run_id: worker.clone(),
        recovery: Some(cockpit_protocol::projects::WorkspaceRecoveryAction::AcceptExistingWorktree),
    }).unwrap();
    let recovery = fixture.run(&worker);
    assert_eq!(recovery.dispatch.as_ref().unwrap().step, DispatchStep::SetupUnknown);
    assert_eq!(recovery.dispatch.as_ref().unwrap().recovery,
        Some(cockpit_protocol::projects::WorkspaceRecoveryAction::AcceptExistingWorktree));
}

#[test]
fn old_unknown_launch_with_fresh_bound_agent_proof_stays_quiet_before_reconcile() {
    for evidence in 0..5 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let task = fixture.create_task(&root, "Recover an old launch outcome");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        fixture.launch_and_bind(&worker);
        fixture.seed_run(&worker, |run| {
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.step = DispatchStep::LaunchUnknown;
            dispatch.agent_started = false;
            dispatch.error = Some(ErrorResponse {
                code: "agent_start_unobserved".into(), message: "Prior installation did not recognize this bound launch".into(),
            });
            dispatch.updated_at = (time::OffsetDateTime::now_utc() - time::Duration::hours(2))
                .format(&time::format_description::well_known::Rfc3339).unwrap();
        });
        let location = fixture.run(&worker).attachment.unwrap().location.unwrap();
        let runtime = herdr::RuntimeView {
            endpoint_identity: location.endpoint_identity.clone(), boot_id: location.boot_id.clone(),
            workspaces: Vec::new(),
            panes: vec![herdr::RuntimePane {
                workspace_id: location.workspace_id.clone(), workspace_label: "Project".into(),
                tab_id: location.tab_id.clone(), tab_label: "Worker".into(), pane_id: location.pane_id.clone(),
                terminal_id: location.terminal_id.clone(),
                native_session_id: if evidence == 4 { Some("replacement-native-session".into()) } else { location.native_session_id.clone() },
                native_session_file: None,
                agent_name: if evidence == 1 { Some("changed-display-name".into()) } else { None },
                agent_kind: if evidence == 3 { None } else { Some("service-test".into()) },
                launch_pending: evidence == 2,
                agent_status: Some("working".into()), state_changed_at: None,
            }, observed_root_pane(&fixture, &root)],
        };
        let bytes = fixture.state_bytes();
        fixture.service.record_observation(&runtime, SESSION).unwrap();
        let failures = dispatch_events(&fixture, &root, "dispatch_failure");
        if evidence <= 1 {
            assert!(failures.is_empty());
            assert_eq!(fixture.state_bytes(), bytes);
            fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
            assert_eq!(fixture.run(&worker).dispatch.as_ref().unwrap().step, DispatchStep::Launched);
            assert!(dispatch_events(&fixture, &root, "dispatch_recovered").is_empty());
        } else {
            assert_eq!(failures.len(), 1);
            let bytes = fixture.state_bytes();
            fixture.service.record_observation(&runtime, SESSION).unwrap();
            assert_eq!(fixture.state_bytes(), bytes);
        }
    }
}
