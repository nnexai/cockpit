use super::*;

#[test]
fn staged_private_credential_requires_publication_then_allows_credential_only_authority() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, mut actor) = fixture.working(&root);
    let (record, path) = credentials::stage_credential(fixture.service.base(), &fixture.run(&worker), JobRole::Worker).unwrap();
    caller_mut(&mut actor).auth = credentials::read_job_auth(&path).unwrap();
    let action = report(ReportKind::Progress, None, None);
    let before = fixture.state_bytes();
    assert_code(fixture.apply(&actor, action.clone()), "job_unauthorized");
    assert_eq!(fixture.state_bytes(), before);
    fixture.seed_state(|state| state.credentials.push(record));
    fixture.apply(&actor, action).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
}

#[test]
fn credential_role_root_epoch_and_exclusive_checkout_claim_are_all_fenced() {
    for fence in 0..8 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (_, worker, actor) = fixture.working(&root);
        let Actor::Agent(caller) = &actor else { unreachable!() };
        fixture.seed_state(|state| {
            let index = run_index(state, &worker).unwrap();
            let claim_id = state.runs[index].checkout_claim_id.clone().unwrap();
            let claim_index = state.checkout_claims.iter().position(|c| c.claim_id == claim_id).unwrap();
            let credential = state.credentials.iter_mut()
                .find(|record| record.credential_id == caller.auth.credential_id).unwrap();
            match fence {
                0 => credential.state = credentials::CredentialState::Revoked,
                1 => credential.root_id = id(),
                2 => credential.role = JobRole::Root,
                3 => state.runs[index].attachment.as_mut().unwrap().epoch += 1,
                4 => state.checkout_claims[claim_index].state = claims::ClaimState::Released,
                5 => state.checkout_claims[claim_index].holder.as_mut().unwrap().run_id = id(),
                6 => state.runs[index].checkout.as_mut().unwrap().directory_identity.inode = "replaced-directory".into(),
                7 => {
                    let mut competing = state.checkout_claims[claim_index].clone();
                    competing.claim_id = id();
                    state.checkout_claims.push(competing);
                }
                _ => unreachable!(),
            }
        });
        let before = fixture.state_bytes();
        assert_code(fixture.apply(&actor, report(ReportKind::Progress, None, None)), "job_unauthorized");
        assert_eq!(fixture.state_bytes(), before, "Fence {fence} must not write");
        assert!(fixture.run(&worker).result.is_none());
    }
}


#[test]
fn run_resume_identical_replay_preserves_protected_writer_and_changed_payload_conflicts() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let other_root = fixture.root();
    let original = fixture.run(&worker);
    let old_claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| original.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    let action = OrchestrationAction::RunResume { run_id: worker.clone(), operation_id: id(), expected_attachment_epoch: Some(original.attachment.as_ref().unwrap().epoch), confirmed_stopped_attachment_id: None };
    let mut receiver = fixture.service.subscribe();
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::RunResumed { resume: first } = first.result else {
        panic!("Expected resume receipt");
    };
    assert!(!first.duplicate);
    assert_eq!(first.resume.phase, ResumePhase::Blocked);
    assert_eq!(first.resume.cause.as_ref().unwrap().code, "writer_exit_unproven");
    assert_eq!(first.resume.cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
    assert!(first.resume.decision_id.is_none(), "Missing observation is not a living-writer replacement decision");
    let state = fixture.state();
    let operation = state.operations.iter().find(|record| record.operation_id == first.resume.operation_id).unwrap();
    assert_eq!(operation.run_id.as_deref(), Some(worker.as_str()));
    assert_eq!(operation.resume_operation_id.as_deref(), Some(first.resume.operation_id.as_str()));
    assert!(operation.result.is_none());
    assert!(operation.effect_receipts.is_empty());
    let blocked = fixture.run(&worker);
    assert_eq!(serde_json::to_value(blocked.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
    assert_eq!(blocked.checkout_claim_id, original.checkout_claim_id);
    let current_claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| claim.claim_id == old_claim.claim_id).unwrap();
    assert_eq!(serde_json::to_value(current_claim).unwrap(), serde_json::to_value(old_claim).unwrap());
    let before = fixture.state_bytes();
    let revision = fixture.state().revision;
    assert!(receiver.has_changed().unwrap(), "The first resume operation must publish its commit");
    assert_eq!(*receiver.borrow_and_update(), revision);
    let replay = fixture.operator(action.clone());
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else {
        panic!("Expected resume replay");
    };
    assert!(replay.duplicate);
    assert_eq!(serde_json::to_value(replay.resume).unwrap(), serde_json::to_value(first.resume).unwrap());
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().revision, revision);
    assert!(!receiver.has_changed().unwrap());
    for field in 0..2 {
        let mut changed = action.clone();
        let OrchestrationAction::RunResume { run_id, expected_attachment_epoch, .. } = &mut changed else { unreachable!() };
        match field {
            0 => *expected_attachment_epoch = Some(original.attachment.as_ref().unwrap().epoch + 1),
            1 => *run_id = other_root.clone(),
            _ => unreachable!(),
        }
        assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
        assert_eq!(fixture.state_bytes(), before);
    }
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    assert_eq!(fixture.run(&worker).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
}

#[test]
fn supervisor_start_creates_private_liaison_and_replays_its_operation() {
    let fixture = Fixture::new();
    let action = OrchestrationAction::SupervisorStart {
        operation_id: id(), target: Some(target()),
        label: Some("Coordinator".into()), selected_root_id: None,
    };
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::SupervisorStarted { start: first } = first.result else {
        panic!("Expected supervisor start");
    };
    assert!(!first.duplicate);
    assert_ne!(fixture.run(&first.root_id).project, Some(fixture.project()));
    assert_eq!(fixture.run(&first.root_id).attachment.as_ref().unwrap().epoch, 1);
    assert!(fixture.run(&first.root_id).checkout_claim_id.is_none());
    let before = fixture.state_bytes();
    let replay = fixture.operator(action.clone());
    let OrchestrationActionResult::SupervisorStarted { start: replay } = replay.result else {
        panic!("Expected supervisor replay");
    };
    assert!(replay.duplicate);
    assert_eq!(replay.root_id, first.root_id);
    assert_eq!(fixture.state_bytes(), before);
    let mut changed = action;
    let OrchestrationAction::SupervisorStart { label, .. } = &mut changed else { unreachable!() };
    *label = Some("Changed request under the same operation ID".into());
    assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn concurrent_starts_match_or_create_one_session_liaison() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let barrier = std::sync::Barrier::new(3);
    let revision = fixture.state().revision;
    let roots = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let configuration = &fixture.configuration;
            let project = project.clone();
            let barrier = &barrier;
            let operation_id = id();
            workers.push(scope.spawn(move || {
                let service = OrchestrationService::open(configuration).unwrap();
                barrier.wait();
                let response = service.mutate_resolved(
                    &Actor::Operator(OperatorOrigin::Browser),
                    OrchestrationMutationRequest {
                        session_id: SESSION.into(), expected_revision: Some(revision),
                        action: OrchestrationAction::SupervisorStart {
                            operation_id, target: Some(target()),
                            label: Some("Concurrent coordinator".into()), selected_root_id: None,
                        },
                    },
                    &ResolvedMutationInput { project: Some(project), ..ResolvedMutationInput::default() },
                ).unwrap();
                let OrchestrationActionResult::SupervisorStarted { start } = response.result else {
                    panic!("Expected project start");
                };
                start.root_id
            }));
        }
        barrier.wait();
        workers.into_iter().map(|worker| worker.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(roots[0], roots[1]);
    assert_eq!(fixture.state().runs.iter().filter(|run| run.parent_run_id.is_none()
        && run.stage != RunStage::Closed && run.session_id == SESSION).count(), 1);
    assert_eq!(fixture.state().operations.iter()
        .filter(|record| record.kind == store::DurableOperationKind::SupervisorStart).count(), 2);
}


#[test]
fn semantic_registration_never_replaces_physical_process_or_checkout_claim() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let original = fixture.run(&worker);
    for fence in 0..3 {
        let mut changed = actor.clone();
        let caller = caller_mut(&mut changed);
        match fence {
            0 => caller.process.as_mut().unwrap().start_ticks += 1,
            1 => caller.process = None,
            _ => caller.actual_agent_kind = Some(NativeAgentKind::Other("shell".into())),
        }
        fixture.apply(&changed, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
        let after = fixture.run(&worker);
        assert_eq!(after.attachment.as_ref().unwrap().process, original.attachment.as_ref().unwrap().process);
        assert_eq!(after.attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
        assert_eq!(after.checkout_claim_id, original.checkout_claim_id);
    }
}


#[test]
fn obsolete_schema_is_rejected_without_discarding_or_rewriting_history() {
    let fixture = Fixture::new();
    let source = serde_json::to_vec(&serde_json::json!({
        "schema": 1, "revision": 17, "runs": [{"obsolete": "permission-era state"}],
        "messages": [], "subagents": [], "task_intents": [], "assignment_intents": [],
    })).unwrap();
    fixture.service.store.lock().unwrap().state_dir().write("state.json", &source).unwrap();
    match OrchestrationService::open(&fixture.configuration) {
        Err(error) => assert_eq!(error.code, "orchestration_state_incompatible"),
        Ok(_) => panic!("Unsupported history must not be silently discarded"),
    }
    assert_eq!(fixture.state_bytes(), source);
}

#[test]
fn runtime_composition_does_not_hide_malformed_or_unsupported_store_errors() {
    for (source, code) in [
        (b"not-json".as_slice(), "state_corrupt"),
        (br#"{"schema":99,"revision":17}"#.as_slice(), "orchestration_state_incompatible"),
    ] {
        let fixture = Fixture::new();
        {
            let locked = fixture.service.store.lock().unwrap();
            locked.state_dir().write("state.json", source).unwrap();
        }
        match OrchestrationService::open(&fixture.configuration) {
            Err(error) => assert_eq!(error.code, code),
            Ok(_) => panic!("Malformed or unsupported state must not open"),
        }
        assert_eq!(fixture.state_bytes(), source);
    }
}

#[test]
fn physical_resume_preserves_current_writer_until_current_conversation_registration() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, mut actor) = fixture.working(&root);
    let original = fixture.run(&worker);
    let mut observed = original.attachment.clone().unwrap();
    observed.conversation = None;
    let claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| original.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    fixture.seed_state(|state| {
        state.credentials.retain(|record| record.run_id != worker);
        state.operations.retain(|record| record.kind != store::DurableOperationKind::RunRegister
            || record.actor_run_id.as_deref() != Some(worker.as_str()));
        state.runs.iter_mut().find(|run| run.run_id == worker).unwrap()
            .attachment.as_mut().unwrap().conversation = None;
        let ownership = state.task_ownership.iter_mut().find(|record|
            record.root_id == root && Some(&record.task_id) == original.task_id.as_ref()).unwrap();
        ownership.owner = None;
        ownership.generation = 0;
    });
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
    for staged in [false, true] {
        if staged {
            caller_mut(&mut actor).auth = fixture.auth(&worker, JobRole::Worker);
        }
        let response = fixture.resume_observed(&worker, observed.clone());
        assert_eq!(response.resume.phase, ResumePhase::Blocked);
        assert_eq!(response.resume.cause.as_ref().unwrap().code, "run_unregistered");
        assert_eq!(response.resume.cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
        assert!(response.resume.decision_id.is_none());
        let current = fixture.run(&worker);
        let attachment = current.attachment.as_ref().unwrap();
        assert_eq!(attachment.state, AttachmentState::Disconnected);
        assert_eq!(attachment.attachment_id, observed.attachment_id);
        assert_eq!(attachment.epoch, observed.epoch);
        assert_eq!(attachment.process, observed.process);
        assert_eq!(attachment.shell, observed.shell);
        assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
        assert_eq!(serde_json::to_value(&current.dispatch).unwrap(), serde_json::to_value(&original.dispatch).unwrap());
        let protected = fixture.state().checkout_claims.into_iter()
            .find(|record| record.claim_id == claim.claim_id).unwrap();
        assert_eq!(serde_json::to_value(protected).unwrap(), serde_json::to_value(&claim).unwrap());
        assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker),
            "An issued credential alone does not pin a cooperative conversation");
    }
    let blocked = fixture.run(&worker).resume.unwrap();
    fixture.apply(&actor, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
    let connected = fixture.run(&worker);
    assert_eq!(connected.attachment.as_ref().unwrap().state, AttachmentState::Connected);
    assert_eq!(connected.attachment.as_ref().unwrap().epoch, observed.epoch);
    assert_eq!(connected.attachment.as_ref().unwrap().process, observed.process);
    assert_eq!(connected.checkout_claim_id, original.checkout_claim_id);
    let dispatch = connected.dispatch.as_ref().unwrap();
    let previous_dispatch = original.dispatch.as_ref().unwrap();
    assert_eq!(dispatch.launch_tag, previous_dispatch.launch_tag);
    assert_eq!(dispatch.launch_attempt, previous_dispatch.launch_attempt);
    assert_eq!(dispatch.endpoint_identity, previous_dispatch.endpoint_identity);
    assert_eq!(dispatch.agent_started, previous_dispatch.agent_started);
    let resumed = connected.resume.as_ref().unwrap();
    assert_eq!(resumed.operation_id, blocked.operation_id);
    assert_eq!(resumed.phase, ResumePhase::Connected);
    assert!(resumed.cause.is_none(), "Authenticated registration clears unavailable UX immediately");
    assert!(resumed.replacement_attachment_epoch.is_none());
    let state = fixture.state();
    let operation = state.operations.iter().find(|record| record.operation_id == blocked.operation_id).unwrap();
    assert_eq!(operation.run_id.as_deref(), Some(worker.as_str()));
    assert_eq!(operation.resume_operation_id.as_deref(), Some(blocked.operation_id.as_str()));
    assert!(operation.result.is_none());
    assert!(operation.effect_receipts.is_empty());
    let protected = state.checkout_claims.iter().find(|record| record.claim_id == claim.claim_id).unwrap();
    assert_eq!(serde_json::to_value(protected).unwrap(), serde_json::to_value(&claim).unwrap());
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
    let before = fixture.state_bytes();
    let replay = fixture.operator(OrchestrationAction::RunResume { run_id: worker.clone(), operation_id: blocked.operation_id, expected_attachment_epoch: Some(observed.epoch), confirmed_stopped_attachment_id: None });
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.resume.phase, ResumePhase::Connected);
    assert!(replay.resume.cause.is_none());
    assert_eq!(fixture.state_bytes(), before, "Owner replay returns the immediately recovered receipt without relaunch");
}

#[test]
fn physical_start_observation_reuses_credential_bound_root_without_conversation_metadata() {
    for staged in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        fixture.launch_and_bind(&root);
        let original = fixture.run(&root);
        let mut observed = original.attachment.clone().unwrap();
        observed.conversation = None;
        fixture.seed_state(|state| {
            state.credentials.retain(|record| record.run_id != root);
            state.operations.retain(|record| record.kind != store::DurableOperationKind::RunRegister
                || record.actor_run_id.as_deref() != Some(root.as_str()));
            state.runs.iter_mut().find(|run| run.run_id == root).unwrap()
                .attachment.as_mut().unwrap().conversation = None;
        });
        if staged {
            fixture.auth(&root, JobRole::Root);
        }
        let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            OrchestrationMutationRequest {
                session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
                action: OrchestrationAction::SupervisorStart {
                    operation_id: id(), target: Some(target()), label: None,
                    selected_root_id: Some(root.clone()),
                },
            }, &ResolvedMutationInput {
                project: Some(fixture.project()), attachment_observation: Some(observed.clone()),
                ..ResolvedMutationInput::default()
            }).unwrap();
        let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
        assert_eq!(start.root_id, root);
        let current = fixture.run(&root);
        if staged {
            assert_eq!(start.outcome, SupervisorStartOutcome::Reused);
            assert!(current.resume.is_none());
            assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
            assert_eq!(current.attachment.as_ref().unwrap().state, observed.state);
        } else {
            assert_eq!(start.outcome, SupervisorStartOutcome::Resuming);
            let resume = current.resume.as_ref().unwrap();
            assert_eq!(resume.phase, ResumePhase::Blocked);
            assert_eq!(resume.cause.as_ref().unwrap().code, "run_unregistered");
            assert_eq!(resume.cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
            assert_eq!(current.attachment.as_ref().unwrap().state, AttachmentState::Disconnected);
            assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
        }
        let attachment = current.attachment.as_ref().unwrap();
        assert_eq!(attachment.epoch, observed.epoch);
        assert_eq!(attachment.process, observed.process);
        assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
        assert_eq!(fixture.state().runs.len(), 1);
    }
}

#[test]
fn authenticated_current_epoch_attach_allows_physical_reattach_without_launch_or_claim_transfer() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let original = fixture.run(&worker);
    let observed = original.attachment.clone().unwrap();
    let claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| original.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    let (_, _, bindings) = fixture.service.recorded_runs_with_bindings(SESSION).unwrap();
    assert_eq!(bindings.get(&worker), Some(&(observed.attachment_id.clone(), observed.epoch)));
    let result = fixture.resume_observed(&worker, observed.clone());
    assert_eq!(result.resume.phase, ResumePhase::Reattached);
    assert!(result.resume.replacement_attachment_epoch.is_none());
    assert!(result.resume.cause.is_none());
    let current = fixture.run(&worker);
    assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
    assert_eq!(serde_json::to_value(current.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
    assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
    let retained = fixture.state().checkout_claims.into_iter().find(|record| record.claim_id == claim.claim_id).unwrap();
    assert_eq!(serde_json::to_value(retained).unwrap(), serde_json::to_value(claim).unwrap());
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
}

#[test]
fn revoked_credentials_or_stale_attachment_epochs_never_prove_current_registration() {
    for fence in 0..2 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (_, worker, _) = fixture.working(&root);
        let original = fixture.run(&worker);
        let observed = original.attachment.clone().unwrap();
        fixture.seed_state(|state| {
            if fence == 0 {
                for credential in state.credentials.iter_mut().filter(|record| record.run_id == worker) {
                    credential.state = credentials::CredentialState::Revoked;
                }
            } else {
                state.credentials.iter_mut().filter(|record| record.run_id == worker)
                    .for_each(|record| record.attachment_epoch += 1);
            }
        });
        assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
        let result = fixture.resume_observed(&worker, observed.clone());
        assert_eq!(result.resume.phase, ResumePhase::Blocked);
        assert_eq!(result.resume.cause.as_ref().unwrap().code, "run_unregistered");
        assert_eq!(result.resume.cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
        let current = fixture.run(&worker);
        assert_eq!(current.attachment.as_ref().unwrap().epoch, observed.epoch);
        assert_eq!(current.attachment.as_ref().unwrap().process, observed.process);
        assert_eq!(current.attachment.as_ref().unwrap().state, AttachmentState::Disconnected);
        assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
        assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
    }
}

#[test]
fn physical_start_reuses_only_a_root_with_applied_current_epoch_attachment_binding() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.launch_and_bind(&root);
    let original = fixture.run(&root);
    fixture.apply(&actor, OrchestrationAction::RunRegister { operation_id: id() }).unwrap();
    let observed = fixture.run(&root).attachment.unwrap();
    let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
            action: OrchestrationAction::SupervisorStart {
                operation_id: id(), target: Some(target()), label: None,
                selected_root_id: Some(root.clone()),
            },
        }, &ResolvedMutationInput {
            project: Some(fixture.project()), attachment_observation: Some(observed.clone()),
            ..ResolvedMutationInput::default()
        }).unwrap();
    let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
    assert_eq!(start.root_id, root);
    assert_eq!(start.outcome, SupervisorStartOutcome::Reused);
    assert!(start.resume_operation_id.is_none());
    let current = fixture.run(&root);
    assert_eq!(current.attachment.as_ref().unwrap().epoch, observed.epoch);
    assert_eq!(current.attachment.as_ref().unwrap().process, observed.process);
    assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
    assert_eq!(fixture.state().runs.len(), 1);
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
}

#[test]
fn closed_history_with_recorded_terminal_cannot_be_adopted_as_another_root() {
    let fixture = Fixture::new();
    let seed = fixture.root();
    fixture.launch_tab(&seed);
    let mut observed = fixture.run(&seed).attachment.unwrap();
    let mut location = observed.location.clone().unwrap();
    location.tab_id = "fresh-adoption-tab".into();
    location.pane_id = "fresh-adoption-pane".into();
    location.terminal_id = Some("fresh-adoption-terminal".into());
    location.native_session_id = Some("fresh-adoption-native".into());
    observed.location = Some(location.clone());
    observed.process = Some(live_process());
    let adopt = |label: &str| fixture.service.mutate_resolved(
        &Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
            action: OrchestrationAction::RunAdopt { label: label.into(), location: location.clone() },
        },
        &ResolvedMutationInput {
            project: Some(fixture.project()), attachment_observation: Some(observed.clone()),
            ..ResolvedMutationInput::default()
        },
    );
    let adopted = run_result(adopt("Fresh selected terminal").unwrap()).0;
    assert_eq!(fixture.run(&adopted).kind, RunKind::Adopted);
    fixture.seed_run(&adopted, |run| {
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Cancelled);
    });
    let before = fixture.state_bytes();
    assert_eq!(fixture.run(&adopted).attachment.unwrap().process, observed.process,
        "Closing tracking is not proof that its physical process exited");
    assert_code(adopt("Must not duplicate the retained incarnation"), "attachment_conflict");
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn resume_join_replays_current_owner_and_settles_old_receipt_before_later_resume() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, _) = fixture.working(&root);
    let response = fixture.resume_exited(&Actor::Operator(OperatorOrigin::Browser), &worker);
    let OrchestrationActionResult::RunResumed { resume: first } = response.result else { unreachable!() };
    assert_eq!(first.resume.phase, ResumePhase::Launching);
    let joined_action = fixture.resume_action(&worker);
    let joined = fixture.operator(joined_action.clone());
    let OrchestrationActionResult::RunResumed { resume: joined } = joined.result else { unreachable!() };
    assert_eq!(joined.resume.operation_id, first.resume.operation_id);
    let OrchestrationAction::RunResume { operation_id: joined_id, .. } = &joined_action else { unreachable!() };
    let state = fixture.state();
    let owner = state.operations.iter().find(|record| record.operation_id == first.resume.operation_id).unwrap();
    assert!(owner.result.is_none());
    assert_eq!(owner.effect_receipts.len(), 2);
    let restoration = cleanup::restoration_summary(&state, SESSION);
    assert_eq!((restoration.reattached, restoration.total_attachments), (0, 1));
    let join = state.operations.iter().find(|record| &record.operation_id == joined_id).unwrap();
    assert_eq!(join.run_id.as_deref(), Some(worker.as_str()));
    assert_eq!(join.resume_operation_id.as_deref(), Some(first.resume.operation_id.as_str()));
    assert!(join.result.is_none());
    assert!(join.effect_receipts.is_empty());
    fixture.launch_and_bind(&worker);
    assert_eq!(fixture.run(&worker).resume.as_ref().unwrap().phase, ResumePhase::Connected);
    let restoration = cleanup::restoration_summary(&fixture.state(), SESSION);
    assert_eq!((restoration.reattached, restoration.total_attachments), (1, 1));
    let bytes = fixture.state_bytes();
    let replay = fixture.operator(joined_action.clone());
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.resume.phase, ResumePhase::Connected);
    assert_eq!(fixture.state_bytes(), bytes);
    let later = fixture.operator(fixture.resume_action(&worker));
    let OrchestrationActionResult::RunResumed { resume: later } = later.result else { unreachable!() };
    assert_ne!(later.resume.operation_id, first.resume.operation_id);
    assert_eq!(later.resume.phase, ResumePhase::Blocked);
    let state = fixture.state();
    for operation_id in [&first.resume.operation_id, joined_id] {
        let operation = state.operations.iter().find(|record| &record.operation_id == operation_id).unwrap();
        let Some(OrchestrationActionResult::RunResumed { resume }) = &operation.result else { unreachable!() };
        assert_eq!(resume.resume.operation_id, first.resume.operation_id);
        assert_eq!(resume.resume.phase, ResumePhase::Connected);
    }
    let bytes = fixture.state_bytes();
    let replay = fixture.operator(joined_action);
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.resume.operation_id, first.resume.operation_id);
    assert_eq!(replay.resume.phase, ResumePhase::Connected);
    assert_eq!(fixture.state_bytes(), bytes);
    let state = fixture.state();
    let owner = state.operations.iter().find(|record| record.operation_id == first.resume.operation_id).unwrap();
    assert_eq!(owner.effect_receipts.len(), 2, "Replays never rearchive launch/setup effects");
}

#[test]
fn worker_resume_of_own_job_blocks_without_stopping_same_boot_writer() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let action = fixture.resume_action(&worker);
    let response = fixture.apply(&actor, action).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
    assert_eq!(resume.resume.phase, ResumePhase::Blocked);
    let state = fixture.state();
    let operation = state.operations.iter().find(|record| record.operation_id == resume.resume.operation_id).unwrap();
    assert!(operation.result.is_none());
    assert!(operation.effect_receipts.is_empty());
    assert!(state.checkout_claims.iter().all(|claim| claim.state != claims::ClaimState::Released));
    assert!(state.credentials.iter().filter(|credential| credential.run_id == worker)
        .all(|credential| credential.state == credentials::CredentialState::Active));
}

#[test]
fn registration_after_physical_launch_proof_starts_once_and_sends_one_brief() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Late private attachment");
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let actor = fixture.launch_tab(&worker);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    // Physical runtime proof does not register a cooperative conversation.
    fixture.seed_run(&worker, |run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.process = caller.process.clone();
    });
    fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Starting);
    assert!(fixture.state().messages.iter().all(|message|
        message.to_run_id != worker || message.kind != MessageKind::WorkBrief));
    let attach = OrchestrationAction::RunRegister { operation_id: id() };
    fixture.apply(&actor, attach.clone()).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Working);
    let bytes = fixture.state_bytes();
    fixture.apply(&actor, attach).unwrap();
    assert_eq!(fixture.state_bytes(), bytes);
    let briefs = fixture.state().messages.into_iter().filter(|message|
        message.to_run_id == worker && message.kind == MessageKind::WorkBrief).collect::<Vec<_>>();
    assert_eq!(briefs.len(), 1);
    assert_eq!(briefs[0].text, fixture.run(&worker).brief);
}

fn restoration_resume(run: &Run, phase: ResumePhase) -> ResumeRecord {
    ResumeRecord {
        operation_id: format!("resume-{}", run.run_id),
        run_id: run.run_id.clone(),
        previous_attachment_epoch: run.attachment.as_ref().map(|attachment| attachment.epoch),
        phase,
        cause: None,
        decision_id: None,
        replacement_attachment_epoch: None,
        updated_at: run.updated_at.clone(),
    }
}

fn restoration_resource(run: &Run, cleanup_state: CleanupState) -> OwnedResourceRecord {
    OwnedResourceRecord {
        resource_id: format!("resource-{}", run.run_id),
        root_id: run.root_id.clone(),
        run_id: run.run_id.clone(),
        kind: OwnedResourceKind::Space,
        ownership: ResourceOwnership::Owned,
        creation_operation_id: Some("creation".into()),
        identity: OwnedResourceIdentity::Space {
            identity: OwnedSpaceIdentity {
                creation_operation_id: "creation".into(),
                endpoint_identity: "endpoint".into(),
                session_id: run.session_id.clone(),
                workspace_id: "space".into(),
                checkout_path: "/isolated-checkout".into(),
            },
        },
        authority: CleanupAuthority::OwnedWhenSafe,
        consumers: Vec::new(),
        integration_result_id: None,
        cleanup: CleanupItem {
            operation_id: None,
            state: cleanup_state,
            reason: None,
            effects: Vec::new(),
            updated_at: run.updated_at.clone(),
        },
    }
}

#[test]
fn restoration_counts_ignore_completed_worker_history_and_unresumed_attachments() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut state = fixture.state();
    let template = fixture.run(&root);
    assert!(template.attachment.is_some());
    for stage in [RunStage::Closed, RunStage::Reported] {
        for phase in [
            None,
            Some(ResumePhase::Reconciling),
            Some(ResumePhase::Launching),
            Some(ResumePhase::Reattached),
            Some(ResumePhase::Connected),
            Some(ResumePhase::Blocked),
        ] {
            let mut worker = template.clone();
            worker.run_id = format!("history-{stage:?}-{phase:?}");
            worker.kind = RunKind::Worker;
            worker.parent_run_id = Some(root.clone());
            worker.stage = stage;
            worker.resume = phase.map(|phase| restoration_resume(&worker, phase));
            state.runs.push(worker);
        }
    }
    let mut completed = template;
    completed.run_id = "result-bearing-working-worker".into();
    completed.kind = RunKind::Worker;
    completed.stage = RunStage::Working;
    completed.resume = Some(restoration_resume(&completed, ResumePhase::Launching));
    completed.result = Some(Report {
        message_id: "completed-result".into(),
        kind: ReportKind::Result,
        outcome: Some(ReportOutcome::Succeeded),
        summary: "Completed work retained before its stage catches up".into(),
        plan: None,
        source_output: None,
        at: completed.updated_at.clone(),
    });
    state.runs.push(completed);
    let restoration = cleanup::restoration_summary(&state, SESSION);
    assert_eq!((restoration.reattached, restoration.total_attachments, restoration.cleanup_pending), (0, 0, 0));
}

#[test]
fn restoration_counts_blocked_root_as_actionable_not_reconnecting() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut state = fixture.state();
    let index = run_index(&state, &root).unwrap();
    let run = &mut state.runs[index];
    let mut resume = restoration_resume(run, ResumePhase::Blocked);
    resume.cause = Some(ResumeCause {
        code: "consumer_activity_unproven".into(),
        message: "Consumer activity cannot be proven".into(),
        class: ResumeCauseClass::Recover,
    });
    run.resume = Some(resume);
    let restoration = cleanup::restoration_summary(&state, SESSION);
    assert_eq!((restoration.reattached, restoration.total_attachments, restoration.cleanup_pending), (0, 0, 0));
    assert_eq!(state.runs[run_index(&state, &root).unwrap()].resume.as_ref().unwrap()
        .cause.as_ref().unwrap().code, "consumer_activity_unproven");
}

#[test]
fn restoration_counts_in_progress_and_settled_root_from_same_recovery_population() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut state = fixture.state();
    let index = run_index(&state, &root).unwrap();
    // Recovery may be reconciling before any replacement attachment exists.
    state.runs[index].attachment = None;
    for (phase, completed) in [
        (ResumePhase::Reconciling, 0),
        (ResumePhase::Launching, 0),
        (ResumePhase::Reattached, 1),
        (ResumePhase::Connected, 1),
    ] {
        state.runs[index].resume = Some(restoration_resume(&state.runs[index], phase));
        let restoration = cleanup::restoration_summary(&state, SESSION);
        assert_eq!((restoration.reattached, restoration.total_attachments), (completed, 1), "{phase:?}");
    }
}

#[test]
fn restoration_cleanup_counts_only_executable_pending_work() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut state = fixture.state();
    let index = run_index(&state, &root).unwrap();
    let resource = restoration_resource(&state.runs[index], CleanupState::Pending);
    state.owned_resources.push(resource);
    let restoration = cleanup::restoration_summary(&state, SESSION);
    assert_eq!(restoration.cleanup_pending, 0, "Active registration is not finishing cleanup");
    state.runs[index].stage = RunStage::Closed;
    for (cleanup_state, pending) in [
        (CleanupState::Pending, 1),
        (CleanupState::Running, 1),
        (CleanupState::Reconciling, 1),
        (CleanupState::Waiting, 0),
        (CleanupState::WaitingIntegration, 0),
        (CleanupState::Stalled, 0),
        (CleanupState::Unknown, 0),
        (CleanupState::Removed, 0),
        (CleanupState::Kept, 0),
    ] {
        state.owned_resources[0].cleanup.state = cleanup_state;
        assert_eq!(cleanup::restoration_summary(&state, SESSION).cleanup_pending, pending, "{cleanup_state:?}");
    }
    state.owned_resources[0].cleanup.state = CleanupState::Pending;
    for ownership in [ResourceOwnership::Borrowed, ResourceOwnership::Unknown] {
        state.owned_resources[0].ownership = ownership;
        assert_eq!(cleanup::restoration_summary(&state, SESSION).cleanup_pending, 0);
    }
    state.owned_resources[0].ownership = ResourceOwnership::Owned;
    state.owned_resources[0].authority = CleanupAuthority::AuthorityUnproven;
    assert_eq!(cleanup::restoration_summary(&state, SESSION).cleanup_pending, 0);
}

#[test]
fn restoration_counts_are_isolated_to_requested_session() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut state = fixture.state();
    let index = run_index(&state, &root).unwrap();
    state.runs[index].resume = Some(restoration_resume(&state.runs[index], ResumePhase::Connected));
    let template = state.runs[index].clone();
    for phase in [ResumePhase::Reconciling, ResumePhase::Launching, ResumePhase::Reattached, ResumePhase::Connected, ResumePhase::Blocked] {
        let mut other = template.clone();
        other.run_id = format!("other-{phase:?}");
        other.session_id = "another-session".into();
        other.root_id = other.run_id.clone();
        other.resume = Some(restoration_resume(&other, phase));
        state.runs.push(other);
    }
    let mut closed = template;
    closed.run_id = "other-closed".into();
    closed.session_id = "another-session".into();
    closed.stage = RunStage::Closed;
    state.owned_resources.push(restoration_resource(&closed, CleanupState::Running));
    state.runs.push(closed);
    let restoration = cleanup::restoration_summary(&state, SESSION);
    assert_eq!((restoration.reattached, restoration.total_attachments, restoration.cleanup_pending), (1, 1, 0));
    let restoration = cleanup::restoration_summary(&state, "another-session");
    assert_eq!((restoration.reattached, restoration.total_attachments, restoration.cleanup_pending), (2, 4, 1));
    let restoration = cleanup::restoration_summary(&state, "missing-session");
    assert_eq!((restoration.reattached, restoration.total_attachments, restoration.cleanup_pending), (0, 0, 0));
}

#[cfg(target_os = "linux")]
struct RootRecoveryPty {
    controller: std::process::Child,
    shell: NativeShellIdentity,
}

#[cfg(target_os = "linux")]
impl RootRecoveryPty {
    fn new() -> Self {
        use std::io::BufRead;
        let mut controller = std::process::Command::new("python3").args(["-u", "-c", r#"
import os, pty, sys
pid, fd = pty.fork()
if pid == 0:
    os.execlp("sh", "sh", "-c", "printf 'ready\\n'; read ignored")
os.read(fd, 1024)
print(pid, flush=True)
sys.stdin.buffer.read()
os.close(fd)
os.waitpid(pid, 0)
"#]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(controller.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let pid: i32 = line.trim().parse().unwrap();
        let start = crate::process_identity::start_identity(pid).unwrap();
        let shell = crate::process_identity::executable_identity(pid, start).unwrap();
        assert!(crate::process_identity::incarnation_foreground(pid, start).unwrap());
        Self { controller, shell }
    }

    fn finish(&mut self) {
        self.controller.stdin.take();
        self.controller.wait().unwrap();
        assert!(!crate::process_identity::same_machine_process_running(&self.shell.process,
            crate::process_identity::machine_id().as_deref()).unwrap());
    }
}

#[cfg(target_os = "linux")]
impl Drop for RootRecoveryPty {
    fn drop(&mut self) {
        self.controller.stdin.take();
        let _ = self.controller.wait();
    }
}

#[cfg(target_os = "linux")]
struct RootRecoveryHerdr {
    runtime: herdr::RuntimeView,
    process: Option<herdr::PaneProcessInfo>,
    unavailable: bool,
    drift_endpoint: bool,
    runtime_reads: std::sync::atomic::AtomicUsize,
}

#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl herdr::OrchestrationHerdr for RootRecoveryHerdr {
    async fn runtime(&self, _: &str) -> Result<herdr::RuntimeView, InspectionError> {
        if self.unavailable { return Err(error("offline", "Fixture observation is unavailable")); }
        let mut runtime = self.runtime.clone();
        if self.drift_endpoint && self.runtime_reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 0 {
            runtime.endpoint_identity = "changed-between-reads".into();
        }
        Ok(runtime)
    }
    async fn pane_process_info(&self, _: &str, endpoint: &str, pane: &str) -> Result<herdr::PaneProcessInfo, InspectionError> {
        assert_eq!(endpoint, self.runtime.endpoint_identity);
        let info = self.process.clone().ok_or_else(|| error("unavailable", "No fixture process evidence"))?;
        assert_eq!(pane, info.pane_id);
        Ok(info)
    }
    async fn create_agent_tab(&self, _: &str, _: &herdr::AgentTabRequest) -> Result<RunLocation, InspectionError> {
        panic!("Root observation never creates or reuses a terminal")
    }
    async fn create_direct_agent_tab(&self, _: &str, _: &herdr::DirectAgentTabRequest) -> Result<RunLocation, InspectionError> {
        panic!("Root recovery reservation is not a physical launch receipt")
    }
    async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
        panic!("Root recovery never retires its original terminal")
    }
}

#[cfg(target_os = "linux")]
fn root_recovery_binding(fixture: &Fixture, run_id: &str, process: NativeProcessIdentity, shell: NativeShellIdentity) -> Actor {
    if fixture.run(run_id).setup.is_none() { fixture.setup(run_id); }
    let mut actor = fixture.launch_tab(run_id);
    caller_mut(&mut actor).process = Some(process.clone());
    fixture.seed_run(run_id, |run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.machine_id = crate::process_identity::machine_id();
        // Registration grants semantic authority, never physical process evidence.
        // This helper supplies the independently observed saved incarnation.
        attachment.process = Some(process);
        attachment.shell = Some(shell);
    });
    fixture.service.record_launch_pending(&fixture.run(run_id)).unwrap();
    let action = OrchestrationAction::RunRegister { operation_id: id() };
    fixture.apply(&actor, action).unwrap();
    fixture.service.record_launch_verified(&fixture.run(run_id)).unwrap();
    actor
}

#[cfg(target_os = "linux")]
fn root_recovery_runtime(run: &Run, shell: &NativeShellIdentity) -> RootRecoveryHerdr {
    let location = run.attachment.as_ref().unwrap().location.as_ref().unwrap();
    RootRecoveryHerdr {
        runtime: herdr::RuntimeView {
            endpoint_identity: "fresh-root-endpoint".into(), boot_id: Some("fresh-herdr-boot".into()),
            peer_process: None,
            focused_workspace_id: None,
            workspaces: vec![herdr::RuntimeWorkspace { workspace_id: location.workspace_id.clone(), label: "Root Space".into(),
                cwd: run.setup.as_ref().unwrap().checkout_path.clone() }],
            panes: vec![herdr::RuntimePane {
                cwd: None,
                workspace_id: location.workspace_id.clone(), workspace_label: "Root Space".into(),
                tab_id: "fresh-root-tab".into(), tab_label: location.launch_tag.clone(), pane_id: "fresh-root-pane".into(),
                terminal_id: location.terminal_id.clone(), native_session_id: location.native_session_id.clone(),
                native_session_file: None,
                agent_name: Some(location.launch_tag.clone()), agent_kind: Some(run.launch_config.harness_kind.clone()),
                launch_pending: false, agent_status: Some("idle".into()), state_changed_at: None,
            }],
        },
        process: Some(herdr::PaneProcessInfo {
            pane_id: "fresh-root-pane".into(), shell_pid: Some(shell.process.pid), foreground_pgid: Some(shell.process.pid),
            processes: vec![(shell.process.pid, "sh".into())], shell_identity: Some(shell.clone()),
        }),
        unavailable: false,
        drift_endpoint: false, runtime_reads: std::sync::atomic::AtomicUsize::new(0),
    }
}

#[cfg(target_os = "linux")]
fn apply_root_recovery(fixture: &Fixture, root: &str, input: &ResolvedMutationInput) -> RunResumeResult {
    let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None, action: fixture.resume_action(root) }, input).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
    resume
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn same_boot_root_replacement_preserves_live_worker_accepted_history_and_old_shell() {
    for container in 0..4 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let mut main = RootRecoveryPty::new();
        let original_shell = RootRecoveryPty::new();
        let restored_shell = RootRecoveryPty::new();
        if container == 3 {
            let (_, checkout) = project_identity::resolve_owned_directory(&fixture.root).unwrap();
            fixture.seed_run(&root, |run| {
                run.checkout = Some(checkout);
                run.attachment.as_mut().unwrap().launch_mode = Some(AttachmentLaunchMode::DirectProcess);
            });
        }
        let root_actor = root_recovery_binding(&fixture, &root, main.shell.process.clone(),
            if container == 3 { main.shell.clone() } else { original_shell.shell.clone() });
        if container == 3 {
            fixture.seed_run(&root, |run| run.setup.as_mut().unwrap().ownership =
                Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory));
        }
        let (accepted_task, history, history_actor) = fixture.working(&root);
        fixture.apply(&history_actor, fixture.successful_result(&history_actor)).unwrap();
        fixture.operator(OrchestrationAction::Accept { run_id: history.clone(), expected_task_revision: accepted_task.task_revision });
        let task = fixture.create_task(&root, "Independent live writer");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        let worker_actor = root_recovery_binding(&fixture, &worker, live_process(), original_shell.shell.clone());
        fixture.initialized(&worker_actor);
        let old = fixture.run(&root);
        let conversation = old.attachment.as_ref().unwrap().conversation.clone();
        assert_ne!(conversation.as_ref().unwrap().session_id.as_str(),
            old.attachment.as_ref().unwrap().location.as_ref().unwrap().native_session_id.as_deref().unwrap());
        let saved = fixture.state();
        let worker_before = serde_json::to_value(fixture.run(&worker)).unwrap();
        let history_before = serde_json::to_value(fixture.run(&history)).unwrap();
        let claims_before = serde_json::to_value(&saved.checkout_claims).unwrap();
        let credentials_before = serde_json::to_value(saved.credentials.iter().filter(|credential| credential.run_id != root).collect::<Vec<_>>()).unwrap();
        let tasks_before = fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap();
        let mut retained_mail = saved.messages.clone();
        for message in retained_mail.iter_mut().filter(|message| message.to_run_id == root
            && matches!(message.kind, MessageKind::WorkBrief | MessageKind::SupervisorBrief)) {
            message.stale = true;
        }
        let mail_before = serde_json::to_value(&retained_mail).unwrap();
        main.finish();
        let mut runtime = root_recovery_runtime(&old, if container == 1 || container == 3 { &restored_shell.shell } else { &original_shell.shell });
        if container == 2 { runtime.runtime.panes.clear(); runtime.process = None; }
        if container == 3 {
            runtime = restored_root_recovery_runtime(&old, &restored_shell.shell);
            let mut unrelated = runtime.runtime.panes[0].clone();
            unrelated.tab_id = "borrowed-user-tab".into();
            unrelated.pane_id = "borrowed-user-pane".into();
            unrelated.tab_label = "User draft".into();
            unrelated.terminal_id = Some("borrowed-user-terminal".into());
            unrelated.native_session_id = Some("independently-live-user-native".into());
            runtime.runtime.panes.push(unrelated);
        }
        let input = fixture.service.observe_root_resume(&runtime, &old).await;
        assert!(input.root_main_recovery.is_some(), "container {container}");
        assert!(input.writer_exit_proof.is_none());
        let resumed = apply_root_recovery(&fixture, &root, &input);
        assert_eq!(resumed.resume.phase, ResumePhase::Launching);
        let replacement = fixture.run(&root);
        assert_eq!(replacement.root_id, old.root_id);
        assert_eq!(replacement.attachment.as_ref().unwrap().epoch, old.attachment.as_ref().unwrap().epoch + 1);
        assert_eq!(replacement.dispatch.as_ref().unwrap().launch_attempt, old.dispatch.as_ref().unwrap().launch_attempt + 1);
        assert_eq!(replacement.stage, RunStage::Starting);
        assert_eq!(replacement.attachment.as_ref().unwrap().conversation, conversation,
            "Physical replacement retains the saved cooperative conversation");
        assert!(replacement.attachment.as_ref().unwrap().process.is_none(), "Reservation is not a current OMP binding");
        assert_eq!(serde_json::to_value(fixture.run(&worker)).unwrap(), worker_before);
        assert_eq!(serde_json::to_value(fixture.run(&history)).unwrap(), history_before);
        let after = fixture.state();
        assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), claims_before);
        assert_eq!(serde_json::to_value(after.credentials.iter().filter(|credential| credential.run_id != root).collect::<Vec<_>>()).unwrap(), credentials_before);
        assert_eq!(serde_json::to_value(&after.messages).unwrap(), mail_before);
        assert_eq!(fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap(), tasks_before);
        assert!(crate::process_identity::same_machine_process_running(&original_shell.shell.process, crate::process_identity::machine_id().as_deref()).unwrap());
        assert!(after.credentials.iter().filter(|credential| credential.run_id == root
            && credential.attachment_epoch == old.attachment.as_ref().unwrap().epoch).all(|credential| credential.state == credentials::CredentialState::Revoked));
        assert_code(fixture.apply(&root_actor, report(ReportKind::Progress, None, None)), "job_unauthorized");
        let revision = fixture.state().revision;
        let joined = apply_root_recovery(&fixture, &root, &input);
        assert_eq!(joined.resume.operation_id, resumed.resume.operation_id);
        assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().epoch, replacement.attachment.as_ref().unwrap().epoch);
        let repeated = fixture.service.resume_owner(&replacement, &ResolvedMutationInput::default()).unwrap();
        assert!(repeated.duplicate);
        assert_eq!(fixture.state().revision, revision + 1, "Only the joining request records a new durable receipt");
        fixture.apply(&worker_actor, report(ReportKind::Progress, None, None)).unwrap();
        fixture.launch_and_bind(&root);
        assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().conversation, conversation,
            "A new native launch identity never replaces the saved conversation");
        assert_eq!(fixture.run(&root).resume.as_ref().unwrap().phase, ResumePhase::Connected);
        assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn root_recovery_refusals_never_revoke_or_duplicate_an_unknown_occupant() {
    for fence in 0..11 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let mut main = RootRecoveryPty::new();
        let shell = RootRecoveryPty::new();
        root_recovery_binding(&fixture, &root, main.shell.process.clone(), shell.shell.clone());
        if fence != 0 { main.finish(); }
        fixture.seed_run(&root, |run| match fence {
            1 => run.attachment.as_mut().unwrap().machine_id = None,
            2 => run.attachment.as_mut().unwrap().machine_id = Some("foreign-machine".into()),
            3 => run.attachment.as_mut().unwrap().process = None,
            9 => run.dispatch.as_mut().unwrap().step = DispatchStep::SetupUnknown,
            _ => {}
        });
        let old = fixture.run(&root);
        let before = fixture.state();
        let mut runtime = root_recovery_runtime(&old, &shell.shell);
        match fence {
            0 => { runtime.runtime.panes.clear(); runtime.process = None; }
            4 => runtime.runtime.panes[0].launch_pending = true,
            5 => runtime.runtime.panes[0].native_session_id = Some("foreign-native".into()),
            6 => { let mut other = runtime.runtime.panes[0].clone(); other.pane_id = "ambiguous".into(); runtime.runtime.panes.push(other); }
            7 => runtime.process.as_mut().unwrap().processes.push((std::process::id(), "different-live-omp".into())),
            8 => runtime.unavailable = true,
            10 => runtime.runtime.panes[0].terminal_id = Some("alias-only-foreign-terminal".into()),
            _ => {}
        }
        let input = fixture.service.observe_root_resume(&runtime, &old).await;
        assert!(input.root_main_recovery.is_none(), "fence {fence}");
        assert!(input.attachment_observation.is_none(), "fence {fence}");
        assert!(input.observation_cause.is_some(), "fence {fence}");
        let blocked = apply_root_recovery(&fixture, &root, &input);
        assert_eq!(blocked.resume.phase, ResumePhase::Blocked);
        let after = fixture.state();
        assert_eq!(serde_json::to_value(&after.credentials).unwrap(), serde_json::to_value(&before.credentials).unwrap());
        assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&before.checkout_claims).unwrap());
        assert_eq!(serde_json::to_value(fixture.run(&root).attachment).unwrap(), serde_json::to_value(old.attachment).unwrap());
        assert_eq!(after.runs.len(), before.runs.len());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn surviving_root_refreshes_settled_endpoint_and_rejects_stale_location_rewind() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let main = RootRecoveryPty::new();
    let shell = RootRecoveryPty::new();
    root_recovery_binding(&fixture, &root, main.shell.process.clone(), shell.shell.clone());
    let original = fixture.run(&root);
    let initial = fixture.resume_observed(&root, original.attachment.clone().unwrap());
    assert_eq!(initial.resume.phase, ResumePhase::Reattached);
    let mut runtime = root_recovery_runtime(&original, &shell.shell);
    let info = runtime.process.as_mut().unwrap();
    info.foreground_pgid = Some(main.shell.process.pid);
    info.processes = vec![(main.shell.process.pid, "test-agent".into())];
    let request = OrchestrationSnapshotRequest { session_id: SESSION.into(), root_id: Some(root.clone()) };
    let before = fixture.service.snapshot(&runtime, &request).await.unwrap();
    let RuntimeObservation::Fresh { runs, .. } = before.runtime else { unreachable!() };
    assert_eq!(runs.iter().find(|run| run.run_id == root).unwrap().presence, Presence::EndpointChanged);
    let input = fixture.service.observe_root_resume(&runtime, &fixture.run(&root)).await;
    assert!(input.root_main_recovery.is_none());
    assert!(input.attachment_observation.is_some());
    let resumed = fixture.service.resume_owner(&fixture.run(&root), &input).unwrap();
    assert_eq!(resumed.resume.operation_id, initial.resume.operation_id);
    assert_eq!(resumed.resume.phase, ResumePhase::Reattached);
    let restored = fixture.run(&root);
    assert_eq!(restored.attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
    assert_eq!(restored.attachment.as_ref().unwrap().process, original.attachment.as_ref().unwrap().process);
    assert_eq!(restored.dispatch.as_ref().unwrap().launch_attempt, original.dispatch.as_ref().unwrap().launch_attempt);
    assert_eq!(restored.dispatch.as_ref().unwrap().endpoint_identity.as_deref(), Some("fresh-root-endpoint"));
    let after = fixture.service.snapshot(&runtime, &request).await.unwrap();
    let RuntimeObservation::Fresh { runs, .. } = after.runtime else { unreachable!() };
    let observed = runs.iter().find(|run| run.run_id == root).unwrap();
    assert_eq!(observed.presence, Presence::Present);
    assert!(observed.actual_agent);
    let repeated = fixture.service.observe_root_resume(&runtime, &restored).await;
    let revision = fixture.state().revision;
    assert!(fixture.service.resume_owner(&restored, &repeated).unwrap().duplicate);
    assert_eq!(fixture.state().revision, revision);
    let mut later_runtime = runtime.runtime.clone();
    later_runtime.endpoint_identity = "even-newer-endpoint".into();
    let later = RootRecoveryHerdr { runtime: later_runtime, process: runtime.process.clone(), unavailable: false,
        drift_endpoint: false, runtime_reads: std::sync::atomic::AtomicUsize::new(0) };
    let fresh = fixture.service.observe_root_resume(&later, &restored).await;
    fixture.service.resume_owner(&restored, &fresh).unwrap();
    let later_saved = fixture.run(&root);
    let bytes = fixture.state_bytes();
    let stale = fixture.service.resume_owner(&later_saved, &input).unwrap_err();
    assert_eq!(stale.code, "attachment_conflict");
    assert_eq!(fixture.state_bytes(), bytes, "Stale same-process proof cannot rewind a newer endpoint");
    let disconnected = ResolvedMutationInput { observation_cause: Some(ResumeCause { code: "offline".into(),
        message: "Fresh endpoint unavailable".into(), class: ResumeCauseClass::Recover }), ..Default::default() };
    let blocked = fixture.service.resume_owner(&later_saved, &disconnected).unwrap();
    assert_eq!(blocked.resume.phase, ResumePhase::Blocked);
    assert_ne!(blocked.resume.operation_id, initial.resume.operation_id);
    let reconnect = fixture.service.observe_root_resume(&later, &fixture.run(&root)).await;
    let resumed = fixture.service.resume_owner(&fixture.run(&root), &reconnect).unwrap();
    assert_eq!(resumed.resume.phase, ResumePhase::Reattached);
    assert_eq!(resumed.resume.operation_id, blocked.resume.operation_id);
    assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn omp_default_resume_relaunches_root_after_saved_unsupported_block() {
    for harness in ["omp", "service-test"] {
        let fixture = Fixture::new();
        let root = fixture.root();
        fixture.seed_run(&root, |run| {
            run.launch_config.harness_kind = harness.into();
            run.launch_config.resume_args = None;
        });
        let mut main = RootRecoveryPty::new();
        let shell = RootRecoveryPty::new();
        let old_actor = root_recovery_binding(&fixture, &root, main.shell.process.clone(), shell.shell.clone());
        fixture.create_task(&root, "Retain this task across same-chat recovery");
        let original = fixture.run(&root);
        let attachment = original.attachment.as_ref().unwrap();
        let conversation = attachment.conversation.clone();
        assert_eq!(conversation.as_ref().unwrap().harness, harness);
        let tasks_before = fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap();
        let credentials_before = serde_json::to_value(&fixture.state().credentials).unwrap();
        let old_credential_id = caller_auth(&old_actor).credential_id.clone();
        main.finish();
        let runtime = root_recovery_runtime(&original, &shell.shell);
        let proof = fixture.service.observe_root_resume(&runtime, &original).await;
        assert!(proof.root_main_recovery.is_some());
        assert!(proof.writer_exit_proof.is_none());
        let blocked_operation = id();
        fixture.seed_run(&root, |run| {
            run.resume = Some(ResumeRecord {
                operation_id: blocked_operation.clone(), run_id: root.clone(),
                previous_attachment_epoch: Some(attachment.epoch), replacement_attachment_epoch: None,
                phase: ResumePhase::Blocked,
                cause: Some(ResumeCause { code: "harness_resume_unsupported".into(),
                    message: "Saved harness had no same-conversation resume arguments".into(), class: ResumeCauseClass::Recover }),
                decision_id: None, updated_at: now(),
            });
        });
        let resumed = apply_root_recovery(&fixture, &root, &proof);
        assert_ne!(resumed.resume.operation_id, blocked_operation);
        assert_eq!(resumed.run_id, root);
        let replacement = fixture.run(&root);
        assert_eq!(replacement.run_id, original.run_id);
        assert_eq!(replacement.root_id, original.root_id);
        assert_eq!(replacement.attachment.as_ref().unwrap().conversation, conversation);
        assert_eq!(fixture.state().runs.len(), 1);
        assert_eq!(fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap(), tasks_before);
        let state = fixture.state();
        let old_credential = state.credentials.iter().find(|record| record.credential_id == old_credential_id).unwrap();
        if harness == "omp" {
            assert_eq!(resumed.resume.phase, ResumePhase::Launching);
            assert!(resumed.resume.cause.is_none());
            assert_eq!(resumed.resume.previous_attachment_epoch, Some(attachment.epoch));
            assert_eq!(resumed.resume.replacement_attachment_epoch, Some(attachment.epoch + 1));
            assert_eq!(replacement.attachment.as_ref().unwrap().epoch, attachment.epoch + 1);
            assert_eq!(replacement.dispatch.as_ref().unwrap().launch_attempt, original.dispatch.as_ref().unwrap().launch_attempt + 1);
            assert_eq!(old_credential.state, credentials::CredentialState::Revoked);
            assert_code(fixture.apply(&old_actor, report(ReportKind::Progress, None, None)), "job_unauthorized");
            let joined = apply_root_recovery(&fixture, &root, &proof);
            assert_eq!(joined.resume.operation_id, resumed.resume.operation_id);
            assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().epoch, attachment.epoch + 1,
                "Joining the pending resume must not rotate the attachment twice");
        } else {
            assert_eq!(resumed.resume.phase, ResumePhase::Blocked);
            assert_eq!(resumed.resume.cause.as_ref().unwrap().code, "harness_resume_unsupported");
            assert!(resumed.resume.replacement_attachment_epoch.is_none());
            assert_eq!(replacement.attachment.as_ref().unwrap().epoch, attachment.epoch);
            assert_eq!(serde_json::to_value(&replacement.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
            assert_eq!(serde_json::to_value(&state.credentials).unwrap(), credentials_before);
            assert_eq!(old_credential.state, credentials::CredentialState::Active);
            assert!(state.credentials.iter().filter(|record| record.run_id == root)
                .all(|record| record.state == credentials::CredentialState::Active));
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pending_resume_requeues_only_supported_effect_free_prelaunch_refusal() {
    assert_effect_free_resume_retry("harness_resume_unsupported").await;
    assert_effect_free_resume_retry("workspace_missing").await;
    assert_effect_free_resume_retry("retained_space_identity_unproven").await;
    assert_effect_free_resume_retry("retained_space_checkout_changed").await;
}

#[cfg(target_os = "linux")]
async fn assert_effect_free_resume_retry(error_code: &str) {
    let fixture = Fixture::new();
    let root = fixture.root();
    fixture.seed_run(&root, |run| {
        run.launch_config.harness_kind = "omp".into();
        run.launch_config.resume_args = None;
    });
    let mut main = RootRecoveryPty::new();
    let shell = RootRecoveryPty::new();
    root_recovery_binding(&fixture, &root, main.shell.process.clone(), shell.shell.clone());
    fixture.create_task(&root, "Preserve the task and reserved capability on upgrade retry");
    let original = fixture.run(&root);
    main.finish();
    let proof = fixture.service.observe_root_resume(&root_recovery_runtime(&original, &shell.shell), &original).await;
    assert!(proof.root_main_recovery.is_some());
    let pending = apply_root_recovery(&fixture, &root, &proof);
    assert_eq!(pending.resume.phase, ResumePhase::Launching);
    let (credential, _) = credentials::stage_credential(fixture.service.base(), &fixture.run(&root), JobRole::Root).unwrap();
    fixture.seed_state(|state| state.credentials.push(credential));
    fixture.service.record_dispatch(&root, DispatchUpdate::Step {
        step: DispatchStep::NeedsReview,
        error: Some(cockpit_protocol::ErrorResponse {
            code: error_code.into(), message: "Previous launch was refused before effects".into(),
        }),
    }).unwrap();
    if error_code == "workspace_missing" {
        fixture.seed_run(&root, |run| {
            run.dispatch.as_mut().unwrap().staging_workspace_id = Some("stale-space".into());
        });
    }
    let stalled = fixture.run(&root);
    let attachment_before = serde_json::to_value(&stalled.attachment).unwrap();
    let credentials_before = serde_json::to_value(&fixture.state().credentials).unwrap();
    let tasks_before = fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap();
    for fault in 0..12 {
        fixture.seed_run(&root, |run| {
            *run = stalled.clone();
            let dispatch = run.dispatch.as_mut().unwrap();
            match fault {
                0 => {}
                1 => dispatch.launch_tag = Some("possibly-created-tab".into()),
                2 => dispatch.latch_state = Some(AgentLatchState::Unknown),
                3 => dispatch.source_location = original.attachment.as_ref().unwrap().location.clone(),
                4 => dispatch.move_operation_id = Some(id()),
                5 => dispatch.scope_unit = Some("possibly-created-scope".into()),
                6 => dispatch.agent_started = true,
                7 => run.attachment.as_mut().unwrap().location = original.attachment.as_ref().unwrap().location.clone(),
                8 => dispatch.launch_attempt += 1,
                9 => dispatch.error.as_mut().unwrap().code = "launch_outcome_unknown".into(),
                10 if error_code != "harness_resume_unsupported" => dispatch.endpoint_identity = Some("possibly-created-endpoint".into()),
                10 => run.launch_config.harness_kind = "unsupported-agent".into(),
                11 => run.attachment.as_mut().unwrap().state = AttachmentState::Connecting,
                _ => unreachable!(),
            }
        });
        let before = fixture.run(&root);
        let joined = apply_root_recovery(&fixture, &root, &ResolvedMutationInput::default());
        assert_eq!(joined.resume.operation_id, pending.resume.operation_id);
        assert_eq!(joined.resume.phase, ResumePhase::Launching);
        let after = fixture.run(&root);
        assert_eq!(after.attachment.as_ref().unwrap().epoch, stalled.attachment.as_ref().unwrap().epoch);
        assert_eq!(serde_json::to_value(&fixture.state().credentials).unwrap(), credentials_before);
        assert_eq!(fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap(), tasks_before);
        assert_eq!(fixture.state().runs.len(), 1);
        if fault == 0 {
            assert_eq!(after.dispatch.as_ref().unwrap().step, DispatchStep::SetupPending);
            assert!(after.dispatch.as_ref().unwrap().error.is_none());
            assert_eq!(after.dispatch.as_ref().unwrap().launch_attempt, stalled.dispatch.as_ref().unwrap().launch_attempt);
            assert!(after.dispatch.as_ref().unwrap().staging_workspace_id.is_none());
            assert_eq!(serde_json::to_value(&after.attachment).unwrap(), attachment_before);
            let repeated = apply_root_recovery(&fixture, &root, &ResolvedMutationInput::default());
            assert_eq!(repeated.resume.operation_id, pending.resume.operation_id);
            assert_eq!(serde_json::to_value(&fixture.run(&root).dispatch).unwrap(), serde_json::to_value(&after.dispatch).unwrap(),
                "A queued replacement is joined without resetting or rotating it again");
        } else {
            assert_eq!(serde_json::to_value(&after.dispatch).unwrap(), serde_json::to_value(&before.dispatch).unwrap(), "fault {fault}");
            assert_eq!(serde_json::to_value(&after.attachment).unwrap(), serde_json::to_value(&before.attachment).unwrap(), "fault {fault}");
        }
    }
}


#[cfg(target_os = "linux")]
#[tokio::test]
async fn manually_resumed_different_root_pid_blocks_until_its_exit_then_uses_same_owner_operation() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut original_main = RootRecoveryPty::new();
    let original_shell = RootRecoveryPty::new();
    root_recovery_binding(&fixture, &root, original_main.shell.process.clone(), original_shell.shell.clone());
    let original = fixture.run(&root);
    original_main.finish();
    let mut manual_main = RootRecoveryPty::new();
    let mut runtime = root_recovery_runtime(&original, &original_shell.shell);
    let info = runtime.process.as_mut().unwrap();
    info.foreground_pgid = Some(manual_main.shell.process.pid);
    info.processes = vec![(manual_main.shell.process.pid, "manually-resumed-omp".into())];
    let refusal = fixture.service.observe_root_resume(&runtime, &original).await;
    assert!(refusal.root_main_recovery.is_none());
    assert!(refusal.attachment_observation.is_none(), "Conversation identity is not saved process authority");
    let blocked = apply_root_recovery(&fixture, &root, &refusal);
    assert_eq!(blocked.resume.phase, ResumePhase::Blocked);
    assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
    assert!(fixture.state().credentials.iter().filter(|credential| credential.run_id == root)
        .all(|credential| credential.state == credentials::CredentialState::Active));
    assert!(crate::process_identity::same_machine_process_running(&manual_main.shell.process,
        crate::process_identity::machine_id().as_deref()).unwrap());
    manual_main.finish();
    let idle = root_recovery_runtime(&fixture.run(&root), &original_shell.shell);
    let proof = fixture.service.observe_root_resume(&idle, &fixture.run(&root)).await;
    assert!(proof.root_main_recovery.is_some());
    let resumed = fixture.service.resume_owner(&fixture.run(&root), &proof).unwrap();
    assert_eq!(resumed.resume.operation_id, blocked.resume.operation_id);
    assert_eq!(resumed.resume.phase, ResumePhase::Launching);
    assert_eq!(fixture.run(&root).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch + 1);
    assert_eq!(fixture.state().runs.len(), 1);
    assert!(crate::process_identity::same_machine_process_running(&original_shell.shell.process,
        crate::process_identity::machine_id().as_deref()).unwrap());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn authenticated_start_reuse_refreshes_root_receipts_and_rejects_stale_start_proof() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let main = RootRecoveryPty::new();
    let shell = RootRecoveryPty::new();
    root_recovery_binding(&fixture, &root, main.shell.process.clone(), shell.shell.clone());
    let original = fixture.run(&root);
    assert!(fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root),
        "Ordinary credential-authenticated registration must exercise the fast reuse predicate");
    let start_request = || OrchestrationMutationRequest {
        session_id: SESSION.into(), expected_revision: None,
        action: OrchestrationAction::SupervisorStart {
            operation_id: id(), target: Some(target()), label: None,
            selected_root_id: Some(root.clone()),
        },
    };
    let operator = Actor::Operator(OperatorOrigin::Browser);
    let location = original.attachment.as_ref().unwrap().location.as_ref().unwrap();
    let mut unchanged = root_recovery_runtime(&original, &shell.shell);
    unchanged.runtime.endpoint_identity = location.endpoint_identity.clone();
    unchanged.runtime.boot_id = location.boot_id.clone();
    unchanged.runtime.panes[0].tab_id = location.tab_id.clone();
    unchanged.runtime.panes[0].pane_id = location.pane_id.clone();
    let info = unchanged.process.as_mut().unwrap();
    info.pane_id = location.pane_id.clone();
    info.foreground_pgid = Some(main.shell.process.pid);
    info.processes = vec![(main.shell.process.pid, "test-agent".into())];
    let input = fixture.service.observe_root_resume(&unchanged, &original).await;
    let response = fixture.service.mutate_resolved(&operator, start_request(), &input).unwrap();
    let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
    assert_eq!(start.outcome, SupervisorStartOutcome::Reused);
    assert_eq!(start.root_id, root);
    assert!(start.resume_operation_id.is_none());
    assert!(fixture.run(&root).resume.is_none(), "Normal Start reuse does not create a recovery operation");
    let mut restored_runtime = root_recovery_runtime(&original, &shell.shell);
    let info = restored_runtime.process.as_mut().unwrap();
    info.foreground_pgid = Some(main.shell.process.pid);
    info.processes = vec![(main.shell.process.pid, "test-agent".into())];
    let refreshed = fixture.service.observe_root_resume(&restored_runtime, &fixture.run(&root)).await;
    let response = fixture.service.mutate_resolved(&operator, start_request(), &refreshed).unwrap();
    let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
    assert_eq!(start.outcome, SupervisorStartOutcome::Resuming);
    assert_eq!(start.root_id, root);
    let saved = fixture.run(&root);
    assert_eq!(saved.resume.as_ref().unwrap().phase, ResumePhase::Reattached);
    assert_eq!(start.resume_operation_id.as_ref(), Some(&saved.resume.as_ref().unwrap().operation_id));
    let fresh_location = saved.attachment.as_ref().unwrap().location.as_ref().unwrap();
    assert_eq!(fresh_location.endpoint_identity, "fresh-root-endpoint");
    assert_eq!(fresh_location.tab_id, "fresh-root-tab");
    assert_eq!(fresh_location.pane_id, "fresh-root-pane");
    assert_eq!(saved.dispatch.as_ref().unwrap().endpoint_identity.as_ref(), Some(&fresh_location.endpoint_identity));
    assert_eq!(saved.dispatch.as_ref().unwrap().launch_attempt, original.dispatch.as_ref().unwrap().launch_attempt);
    assert_eq!(saved.attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
    assert_eq!(saved.attachment.as_ref().unwrap().process, original.attachment.as_ref().unwrap().process);
    let subsequent = fixture.service.observe_root_resume(&restored_runtime, &saved).await;
    assert!(subsequent.attachment_observation.is_some(), "The refreshed dispatch receipt must pass later root eligibility");
    assert!(subsequent.observation_cause.is_none());
    assert_eq!(apply_root_recovery(&fixture, &root, &subsequent).resume.phase, ResumePhase::Reattached);
    let snapshot = fixture.service.snapshot(&restored_runtime, &OrchestrationSnapshotRequest {
        session_id: SESSION.into(), root_id: Some(root.clone()),
    }).await.unwrap();
    let RuntimeObservation::Fresh { runs, .. } = snapshot.runtime else { unreachable!() };
    let observed = runs.iter().find(|run| run.run_id == root).unwrap();
    assert_eq!(observed.presence, Presence::Present);
    assert!(observed.actual_agent);
    let before = fixture.state_bytes();
    // Its observed location now matches, but the expected pre-refresh
    // attachment is stale. Fast Reused must not discard that CAS fence.
    let failure = fixture.service.mutate_resolved(&operator, start_request(), &refreshed).unwrap_err();
    assert_eq!(failure.code, "attachment_conflict");
    assert_eq!(fixture.state_bytes(), before);
    let failure = fixture.service.mutate_resolved(&operator, start_request(), &input).unwrap_err();
    assert_eq!(failure.code, "attachment_conflict", "Old-location proof must not rewind the fresh receipt");
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().runs.len(), 1);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn authenticated_start_reuse_settles_a_blocked_recovery_even_without_a_location_change() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let main = RootRecoveryPty::new();
    let shell = RootRecoveryPty::new();
    root_recovery_binding(&fixture, &root, main.shell.process.clone(), shell.shell.clone());
    let original = fixture.run(&root);
    let refusal = ResolvedMutationInput { observation_cause: Some(ResumeCause {
        code: "resume_observation_unavailable".into(), message: "Temporary observation failure".into(), class: ResumeCauseClass::Recover,
    }), ..Default::default() };
    assert_eq!(apply_root_recovery(&fixture, &root, &refusal).resume.phase, ResumePhase::Blocked);
    let location = original.attachment.as_ref().unwrap().location.as_ref().unwrap();
    let mut live = root_recovery_runtime(&original, &shell.shell);
    live.runtime.endpoint_identity = location.endpoint_identity.clone();
    live.runtime.boot_id = location.boot_id.clone();
    live.runtime.panes[0].tab_id = location.tab_id.clone();
    live.runtime.panes[0].pane_id = location.pane_id.clone();
    let info = live.process.as_mut().unwrap();
    info.pane_id = location.pane_id.clone();
    info.foreground_pgid = Some(main.shell.process.pid);
    info.processes = vec![(main.shell.process.pid, "test-agent".into())];
    let input = fixture.service.observe_root_resume(&live, &fixture.run(&root)).await;
    let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None,
            action: OrchestrationAction::SupervisorStart {
                operation_id: id(), target: Some(target()), label: None,
                selected_root_id: Some(root.clone()),
            },
        }, &input).unwrap();
    let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
    assert_eq!(start.outcome, SupervisorStartOutcome::Resuming);
    let saved = fixture.run(&root);
    assert_eq!(saved.resume.as_ref().unwrap().phase, ResumePhase::Reattached);
    assert!(saved.resume.as_ref().unwrap().cause.is_none());
    assert_eq!(saved.attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
    assert_eq!(saved.dispatch.as_ref().unwrap().launch_attempt, original.dispatch.as_ref().unwrap().launch_attempt);
    assert_eq!(fixture.state().runs.len(), 1);
}

#[cfg(target_os = "linux")]
fn restored_root_recovery_runtime(run: &Run, shell: &NativeShellIdentity) -> RootRecoveryHerdr {
    let location = run.attachment.as_ref().unwrap().location.as_ref().unwrap();
    let mut restored = root_recovery_runtime(run, shell);
    let pane = &mut restored.runtime.panes[0];
    // Stock Herdr restores topology/tag/cwd, not the old PTY or native agent.
    pane.tab_id = location.tab_id.clone();
    pane.pane_id = location.pane_id.clone();
    pane.terminal_id = Some("new-restored-terminal".into());
    pane.native_session_id = None;
    pane.agent_name = None;
    pane.agent_kind = None;
    restored.process.as_mut().unwrap().pane_id = location.pane_id.clone();
    restored
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn restored_root_terminal_requires_exact_dead_main_receipt_checkout_topology_and_idle_shell() {
    for fence in 0..16 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let mut main = RootRecoveryPty::new();
        let shell = RootRecoveryPty::new();
        let (_, checkout) = project_identity::resolve_owned_directory(&fixture.root).unwrap();
        fixture.seed_run(&root, |run| {
            run.checkout = Some(checkout);
            run.attachment.as_mut().unwrap().launch_mode = Some(AttachmentLaunchMode::DirectProcess);
        });
        root_recovery_binding(&fixture, &root, main.shell.process.clone(), main.shell.clone());
        if fence != 0 { main.finish(); }
        fixture.seed_run(&root, |run| match fence {
            10 => run.checkout = None,
            11 => run.setup.as_mut().unwrap().workspace_id = Some("not-the-saved-workspace".into()),
            12 => run.setup.as_mut().unwrap().checkout_path = "/not-the-retained-checkout".into(),
            _ => {}
        });
        let old = fixture.run(&root);
        let before = fixture.state();
        let mut restored = restored_root_recovery_runtime(&old, &shell.shell);
        match fence {
            0 => {
                restored.runtime.panes[0].native_session_id = old.attachment.as_ref().unwrap().location.as_ref().unwrap().native_session_id.clone();
                restored.runtime.panes[0].agent_kind = Some("service-test".into());
                let info = restored.process.as_mut().unwrap();
                info.foreground_pgid = Some(main.shell.process.pid);
                info.processes = vec![(main.shell.process.pid, "test-agent".into())];
            }
            1 => restored.runtime.panes[0].workspace_id = "foreign-workspace".into(),
            2 => restored.runtime.panes[0].tab_id = "changed-tab".into(),
            3 => restored.runtime.panes[0].pane_id = "changed-pane".into(),
            4 => {
                restored.runtime.panes[0].tab_label = "foreign-label".into();
                restored.runtime.panes[0].agent_name = Some(old.attachment.as_ref().unwrap().location.as_ref().unwrap().launch_tag.clone());
            }
            5 => {
                let foreign = fixture.root.join("foreign-cwd");
                std::fs::create_dir(&foreign).unwrap();
                restored.runtime.workspaces[0].cwd = foreign.to_string_lossy().into_owned();
            }
            6 => restored.runtime.panes[0].launch_pending = true,
            7 => restored.runtime.panes[0].native_session_id = Some("different-live-omp".into()),
            8 => {
                let duplicate = restored.runtime.panes[0].clone();
                restored.runtime.panes.push(duplicate);
            }
            9 => restored.process.as_mut().unwrap().processes.push((std::process::id(), "unknown-live-process".into())),
            13 => {
                let location = old.attachment.as_ref().unwrap().location.as_ref().unwrap();
                restored.runtime.endpoint_identity = location.endpoint_identity.clone();
                restored.runtime.boot_id = location.boot_id.clone();
            }
            14 => {
                let duplicate = restored.runtime.workspaces[0].clone();
                restored.runtime.workspaces.push(duplicate);
            }
            15 => restored.runtime.panes[0].agent_kind = Some("foreign-agent".into()),
            _ => {}
        }
        let input = fixture.service.observe_root_resume(&restored, &old).await;
        assert!(input.root_main_recovery.is_none(), "fence {fence}");
        assert!(input.attachment_observation.is_none(), "fence {fence}");
        assert!(input.observation_cause.is_some(), "fence {fence}");
        assert_eq!(apply_root_recovery(&fixture, &root, &input).resume.phase, ResumePhase::Blocked);
        let after = fixture.state();
        assert_eq!(serde_json::to_value(&after.credentials).unwrap(), serde_json::to_value(&before.credentials).unwrap(), "fence {fence}");
        assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&before.checkout_claims).unwrap(), "fence {fence}");
        assert_eq!(serde_json::to_value(fixture.run(&root).attachment).unwrap(), serde_json::to_value(old.attachment).unwrap(), "fence {fence}");
        assert_eq!(after.runs.len(), before.runs.len(), "fence {fence}");
        assert!(crate::process_identity::same_machine_process_running(&shell.shell.process,
            crate::process_identity::machine_id().as_deref()).unwrap(), "The unclaimed restored shell is never stopped");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn restored_root_checkout_and_setup_receipt_changes_invalidate_recovery_before_revocation() {
    for changed_receipt in 0..3 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let mut main = RootRecoveryPty::new();
        let shell = RootRecoveryPty::new();
        let (_, checkout) = project_identity::resolve_owned_directory(&fixture.root).unwrap();
        fixture.seed_run(&root, |run| run.checkout = Some(checkout));
        root_recovery_binding(&fixture, &root, main.shell.process.clone(), main.shell.clone());
        main.finish();
        let old = fixture.run(&root);
        let restored = restored_root_recovery_runtime(&old, &shell.shell);
        let input = fixture.service.observe_root_resume(&restored, &old).await;
        assert!(input.root_main_recovery.as_ref().unwrap().retained_checkout.is_some());
        fixture.seed_run(&root, |run| match changed_receipt {
            0 => run.checkout = None,
            1 => run.setup.as_mut().unwrap().workspace_id = Some("newer-workspace".into()),
            2 => run.setup.as_mut().unwrap().checkout_path = "/newer-checkout".into(),
            _ => unreachable!(),
        });
        let before = fixture.state_bytes();
        let failure = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None,
                action: fixture.resume_action(&root) }, &input).unwrap_err();
        assert!(matches!(failure.code.as_str(), "root_main_exit_unproven" | "consumer_activity_unproven"), "{failure:?}");
        assert_eq!(fixture.state_bytes(), before, "A stale checkout/receipt proof cannot revoke or reserve");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn credential_only_root_recovers_from_exact_native_exit_without_chat_identity() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let mut main = RootRecoveryPty::new();
    let shell = RootRecoveryPty::new();
    root_recovery_binding(&fixture, &root, main.shell.process.clone(), shell.shell.clone());
    fixture.seed_state(|state| {
        state.runs.iter_mut().find(|run| run.run_id == root).unwrap()
            .attachment.as_mut().unwrap().conversation = None;
        for credential in state.credentials.iter_mut().filter(|credential| credential.run_id == root) {
            credential.conversation = None;
        }
    });
    let original = fixture.run(&root);
    main.finish();
    let runtime = root_recovery_runtime(&original, &shell.shell);
    let input = fixture.service.observe_root_resume(&runtime, &original).await;
    assert!(input.root_main_recovery.is_some());
    let resumed = apply_root_recovery(&fixture, &root, &input);
    assert_eq!(resumed.resume.phase, ResumePhase::Launching);
    let replacement = fixture.run(&root);
    assert_eq!(replacement.attachment.as_ref().unwrap().epoch,
        original.attachment.as_ref().unwrap().epoch + 1);
    assert!(replacement.attachment.as_ref().unwrap().conversation.is_none());
    assert!(crate::process_identity::same_machine_process_running(&shell.shell.process,
        original.attachment.as_ref().unwrap().machine_id.as_deref()).unwrap_or(false));
}


#[cfg(target_os = "linux")]
fn unrecorded_root(fixture: &Fixture, root: &str) -> Actor {
    fixture.setup(root);
    let actor = fixture.launch_and_bind(root);
    fixture.seed_run(root, |run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.process = None;
        attachment.shell = None;
        attachment.process_scope = None;
        attachment.machine_id = None;
    });
    actor
}

#[cfg(target_os = "linux")]
fn confirmed_root_action(fixture: &Fixture, root: &str) -> OrchestrationAction {
    let mut action = fixture.resume_action(root);
    let OrchestrationAction::RunResume { confirmed_stopped_attachment_id, .. } = &mut action else { unreachable!() };
    *confirmed_stopped_attachment_id = Some(fixture.run(root).attachment.unwrap().attachment_id);
    action
}

#[cfg(target_os = "linux")]
fn apply_confirmed_root(fixture: &Fixture, action: OrchestrationAction, input: &ResolvedMutationInput) -> Result<OrchestrationMutationResponse, InspectionError> {
    fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None, action }, input)
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn unrecorded_root_operator_confirmation_recovers_and_keeps_tasks() {
    for prior_block in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let actor = unrecorded_root(&fixture, &root);
        let (accepted_task, history, history_actor) = fixture.working(&root);
        fixture.apply(&history_actor, fixture.successful_result(&history_actor)).unwrap();
        fixture.operator(OrchestrationAction::Accept { run_id: history.clone(), expected_task_revision: accepted_task.task_revision });
        let task = fixture.create_task(&root, "Keep this live worker");
        let worker = fixture.start(&root, &task);
        fixture.setup(&worker);
        let worker_actor = fixture.launch_and_bind(&worker);
        fixture.initialized(&worker_actor);
        let old = fixture.run(&root);
        let shell = RootRecoveryPty::new();
        let mut runtime = root_recovery_runtime(&old, &shell.shell);
        runtime.runtime.panes.clear();
        runtime.process = None;
        let tasks_before = fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap();
        if prior_block {
            let saved = fixture.state();
            let input = fixture.service.observe_root_resume(&runtime, &old).await;
            assert_eq!(input.observation_cause.as_ref().unwrap().code, "root_main_identity_unrecorded");
            let blocked = apply_root_recovery(&fixture, &root, &input);
            assert_eq!(blocked.resume.phase, ResumePhase::Blocked);
            let after = fixture.state();
            assert_eq!(serde_json::to_value(&after.credentials).unwrap(), serde_json::to_value(&saved.credentials).unwrap());
            assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&saved.checkout_claims).unwrap());
            assert_eq!(serde_json::to_value(&fixture.run(&root).attachment).unwrap(), serde_json::to_value(&old.attachment).unwrap());
            let mut crossover = confirmed_root_action(&fixture, &root);
            let OrchestrationAction::RunResume { operation_id, .. } = &mut crossover else { unreachable!() };
            *operation_id = blocked.resume.operation_id;
            let bytes = fixture.state_bytes();
            assert_code(apply_confirmed_root(&fixture, crossover, &input), "operation_conflict");
            assert_eq!(fixture.state_bytes(), bytes);
        }
        let before = fixture.state();
        let action = confirmed_root_action(&fixture, &root);
        let input = fixture.service.observe_confirmed_root_stop(&runtime, &fixture.run(&root),
            &old.attachment.as_ref().unwrap().attachment_id).await;
        assert!(input.observation_cause.is_none());
        assert_eq!(input.root_main_recovery.as_ref().unwrap().basis, claims::RootExitBasis::OperatorConfirmed);
        let response = apply_confirmed_root(&fixture, action.clone(), &input).unwrap();
        let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
        assert_eq!(resume.resume.phase, ResumePhase::Launching);
        let replacement = fixture.run(&root);
        assert_eq!(replacement.root_id, root);
        assert_eq!(replacement.attachment.as_ref().unwrap().epoch, old.attachment.as_ref().unwrap().epoch + 1);
        assert_eq!(replacement.dispatch.as_ref().unwrap().launch_attempt, old.dispatch.as_ref().unwrap().launch_attempt + 1);
        let after = fixture.state();
        for run in before.runs.iter().filter(|run| run.run_id != root) {
            assert_eq!(serde_json::to_value(fixture.run(&run.run_id)).unwrap(), serde_json::to_value(run).unwrap());
        }
        assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&before.checkout_claims).unwrap());
        let mut expected_credentials = before.credentials.clone();
        for credential in expected_credentials.iter_mut().filter(|credential| credential.run_id == root
            && credential.attachment_id == old.attachment.as_ref().unwrap().attachment_id
            && credential.attachment_epoch == old.attachment.as_ref().unwrap().epoch) {
            credential.state = credentials::CredentialState::Revoked;
        }
        assert_eq!(serde_json::to_value(&after.credentials).unwrap(), serde_json::to_value(&expected_credentials).unwrap());
        let mut expected_mail = before.messages.clone();
        for message in expected_mail.iter_mut().filter(|message| message.to_run_id == root
            && matches!(message.kind, MessageKind::WorkBrief | MessageKind::SupervisorBrief)) { message.stale = true; }
        assert_eq!(serde_json::to_value(&after.messages).unwrap(), serde_json::to_value(expected_mail).unwrap());
        assert_eq!(fixture.service.store.tasks_dir().read(tasks_md::root_filename(&root).unwrap()).unwrap(), tasks_before);
        let bytes = fixture.state_bytes();
        let replay = apply_confirmed_root(&fixture, action.clone(), &input).unwrap();
        let OrchestrationActionResult::RunResumed { resume } = replay.result else { unreachable!() };
        assert!(resume.duplicate);
        assert_eq!(fixture.state_bytes(), bytes);
        let mut ordinary = action.clone();
        let OrchestrationAction::RunResume { confirmed_stopped_attachment_id, .. } = &mut ordinary else { unreachable!() };
        *confirmed_stopped_attachment_id = None;
        assert_code(apply_confirmed_root(&fixture, ordinary, &input), "operation_conflict");
        let mut stale = action;
        let OrchestrationAction::RunResume { operation_id, .. } = &mut stale else { unreachable!() };
        *operation_id = id();
        assert_code(apply_confirmed_root(&fixture, stale, &input), "attachment_conflict");
        assert_eq!(fixture.state_bytes(), bytes);
        assert_code(fixture.apply(&actor, report(ReportKind::Progress, None, None)), "job_unauthorized");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn stop_confirmation_refusals_write_nothing() {
    for fault in 0..15 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let actor = unrecorded_root(&fixture, &root);
        let shell = RootRecoveryPty::new();
        let old = fixture.run(&root);
        let mut runtime = root_recovery_runtime(&old, &shell.shell);
        runtime.runtime.panes.clear();
        let mut input = fixture.service.observe_confirmed_root_stop(&runtime, &old,
            &old.attachment.as_ref().unwrap().attachment_id).await;
        assert!(input.root_main_recovery.is_some());
        let mut action = confirmed_root_action(&fixture, &root);
        match fault {
            1 => if let OrchestrationAction::RunResume { expected_attachment_epoch, .. } = &mut action { *expected_attachment_epoch = None; },
            2 => if let OrchestrationAction::RunResume { confirmed_stopped_attachment_id, .. } = &mut action { *confirmed_stopped_attachment_id = Some(id()); },
            3 | 4 => fixture.seed_run(&root, |run| run.resume = Some(restoration_resume(run,
                if fault == 3 { ResumePhase::Launching } else { ResumePhase::Reconciling }))),
            5 => fixture.seed_run(&root, |run| run.attachment.as_mut().unwrap().process = Some(live_process())),
            6 => fixture.seed_run(&root, |run| run.attachment.as_mut().unwrap().machine_id = Some("recorded-machine".into())),
            7 => fixture.seed_run(&root, |run| run.attachment.as_mut().unwrap().shell = Some(shell.shell.clone())),
            8 => fixture.seed_state(|state| {
                let (_, checkout) = project_identity::resolve_owned_directory(&fixture.root).unwrap();
                state.checkout_claims.push(claims::CheckoutClaim {
                    claim_id: id(), checkout, revision: 1, state: claims::ClaimState::Held,
                    holder: Some(AttachmentReference { run_id: root.clone(), attachment_id: old.attachment.as_ref().unwrap().attachment_id.clone(),
                        attachment_epoch: old.attachment.as_ref().unwrap().epoch }), transition_operation_id: None,
                });
            }),
            9 => fixture.seed_run(&root, |run| run.kind = RunKind::Worker),
            10 => fixture.seed_run(&root, |run| run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending),
            11 => if let OrchestrationAction::RunResume { confirmed_stopped_attachment_id, .. } = &mut action { *confirmed_stopped_attachment_id = None; },
            14 => input.root_main_recovery.as_mut().unwrap().basis = claims::RootExitBasis::ExactIncarnation,
            _ => {}
        }
        let bytes = fixture.state_bytes();
        let expected = match fault {
            0 => "operator_confirmation_required",
            11..=14 => "root_main_exit_unproven",
            _ => "root_stop_confirmation_unavailable",
        };
        if fault == 0 {
            assert_code(fixture.service.mutate_resolved(&actor, OrchestrationMutationRequest {
                session_id: SESSION.into(), expected_revision: None, action,
            }, &input), expected);
        } else if fault == 12 {
            assert_eq!(fixture.service.resume_owner(&fixture.run(&root), &input).unwrap_err().code, expected);
        } else if fault == 13 {
            let blocked = apply_root_recovery(&fixture, &root, &ResolvedMutationInput::default());
            let bytes = fixture.state_bytes();
            assert_eq!(fixture.service.advance_resume_observed(&root, &blocked.resume.operation_id, &input).unwrap_err().code, expected);
            assert_eq!(fixture.state_bytes(), bytes);
            continue;
        } else {
            assert_code(apply_confirmed_root(&fixture, action, &input), expected);
        }
        assert_eq!(fixture.state_bytes(), bytes, "fault {fault}");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn confirmed_stop_still_requires_fresh_safe_container() {
    for fault in 0..11 {
        let fixture = Fixture::new();
        let root = fixture.root();
        unrecorded_root(&fixture, &root);
        let old = fixture.run(&root);
        let shell = RootRecoveryPty::new();
        let mut runtime = root_recovery_runtime(&old, &shell.shell);
        match fault {
            0 => { runtime.runtime.panes.clear(); runtime.process = None; }
            1 => runtime.process.as_mut().unwrap().processes.push((std::process::id(), "busy-omp".into())),
            2 => runtime.runtime.panes[0].native_session_id = Some("foreign-conversation".into()),
            3 => runtime.runtime.panes[0].launch_pending = true,
            4 | 5 => {
                let mut duplicate = runtime.runtime.panes[0].clone();
                duplicate.pane_id = "other-pane".into();
                duplicate.terminal_id = Some("other-terminal".into());
                if fault == 4 { duplicate.tab_label = "different-label".into(); duplicate.agent_name = None; }
                else { duplicate.native_session_id = None; }
                runtime.runtime.panes.push(duplicate);
            }
            6 => runtime.process.as_mut().unwrap().shell_identity = None,
            7 => runtime.unavailable = true,
            8 => runtime.runtime.panes[0].workspace_id = "foreign-workspace".into(),
            9 => {
                runtime.runtime.panes.clear();
                runtime.drift_endpoint = true;
            }
            // Even an exactly idle shell is not operator-confirmation authority.
            10 => {}
            _ => {}
        }
        let before = fixture.state();
        let input = fixture.service.observe_confirmed_root_stop(&runtime, &old,
            &old.attachment.as_ref().unwrap().attachment_id).await;
        let response = apply_confirmed_root(&fixture, confirmed_root_action(&fixture, &root), &input).unwrap();
        let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
        if fault == 0 {
            assert_eq!(resume.resume.phase, ResumePhase::Launching);
        } else {
            assert_eq!(resume.resume.phase, ResumePhase::Blocked, "fault {fault}");
            assert!(matches!(resume.resume.cause.as_ref().unwrap().code.as_str(),
                "consumer_activity_unproven" | "resume_observation_unavailable"));
            assert!(project_identity::root_stop_confirmable(&fixture.run(&root)));
            assert_eq!(serde_json::to_value(&fixture.state().credentials).unwrap(), serde_json::to_value(&before.credentials).unwrap());
            assert_eq!(serde_json::to_value(fixture.run(&root).attachment).unwrap(), serde_json::to_value(old.attachment).unwrap());
        }
    }
}
