use super::*;

#[test]
fn staged_private_credential_grants_nothing_until_published_under_the_state_lock() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, mut actor) = fixture.working(&root);
    let (record, path) = credentials::stage_credential(
        fixture.service.base(), &fixture.run(&worker), JobRole::Worker, None,
    ).unwrap();
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
fn run_attach_identical_replay_is_no_write_and_changed_payload_conflicts() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let action = OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    };
    let mut receiver = fixture.service.subscribe();
    let first = fixture.apply(&actor, action.clone()).unwrap();
    let OrchestrationActionResult::RunAttached { attach: first } = first.result else {
        panic!("Expected attachment receipt");
    };
    assert!(!first.duplicate);
    let before = fixture.state_bytes();
    let revision = fixture.state().revision;
    assert!(receiver.has_changed().unwrap(), "The first attachment operation must publish its commit");
    assert_eq!(*receiver.borrow_and_update(), revision);
    let replay = fixture.apply(&actor, action.clone()).unwrap();
    let OrchestrationActionResult::RunAttached { attach: replay } = replay.result else {
        panic!("Expected attachment replay");
    };
    assert!(replay.duplicate);
    assert_eq!(replay.run_id, worker);
    assert_eq!(serde_json::to_value(replay.attachment).unwrap(), serde_json::to_value(first.attachment).unwrap());
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().revision, revision);
    assert!(!receiver.has_changed().unwrap());
    for field in 0..3 {
        let mut changed = action.clone();
        let OrchestrationAction::RunAttach { main_session_id, expected_attachment_epoch, attachment_id, .. } = &mut changed else { unreachable!() };
        match field {
            0 => *main_session_id = "changed-conversation".into(),
            1 => *expected_attachment_epoch += 1,
            2 => *attachment_id = id(),
            _ => unreachable!(),
        }
        assert_code(fixture.apply(&actor, changed), "operation_conflict");
        assert_eq!(fixture.state_bytes(), before);
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
    let action = OrchestrationAction::RunResume {
        run_id: worker.clone(), operation_id: id(),
        expected_attachment_epoch: Some(original.attachment.as_ref().unwrap().epoch),
        mode: ResumeMode::ReattachOrRestart, decision_id: None,
    };
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
    for field in 0..4 {
        let mut changed = action.clone();
        let OrchestrationAction::RunResume { run_id, expected_attachment_epoch, mode, decision_id, .. } = &mut changed else { unreachable!() };
        match field {
            0 => *expected_attachment_epoch = Some(original.attachment.as_ref().unwrap().epoch + 1),
            1 => *mode = ResumeMode::ReplaceExisting,
            2 => *decision_id = Some(id()),
            3 => *run_id = other_root.clone(),
            _ => unreachable!(),
        }
        assert_code(fixture.apply(&Actor::Operator(OperatorOrigin::Browser), changed), "operation_conflict");
        assert_eq!(fixture.state_bytes(), before);
    }
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    assert_eq!(fixture.run(&worker).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
}

#[test]
fn supervisor_start_another_uses_immutable_project_and_replays_its_operation() {
    let fixture = Fixture::new();
    let action = OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::Another, target: Some(target()),
        label: Some("Explicit another coordinator".into()), selected_root_id: None,
        expected_project_key: Some(fixture.project().key),
    };
    let first = fixture.operator(action.clone());
    let OrchestrationActionResult::SupervisorStarted { start: first } = first.result else {
        panic!("Expected supervisor start");
    };
    assert!(!first.duplicate);
    assert_eq!(fixture.run(&first.root_id).project, Some(fixture.project()));
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
fn concurrent_reuse_starts_match_or_create_one_open_project_root() {
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
                            operation_id, mode: SupervisorStartMode::ReuseOrResume, target: Some(target()),
                            label: Some("Concurrent project coordinator".into()), selected_root_id: None,
                            expected_project_key: Some(project.key.clone()),
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
        && run.stage != RunStage::Closed && run.project.as_ref() == Some(&project)).count(), 1);
    assert_eq!(fixture.state().operations.iter()
        .filter(|record| record.kind == store::DurableOperationKind::SupervisorStart).count(), 2);
}

#[test]
fn child_registration_requires_physical_identity_and_replays_without_reminting_credentials() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, parent) = fixture.working(&root);
    let mut registering = parent.clone();
    let caller = caller_mut(&mut registering);
    caller.agent_kind = Some(AgentKind::Subagent);
    caller.subagent_id = Some("native-child".into());
    caller.omp_session_id = Some("native-child-conversation".into());
    let action = OrchestrationAction::JobChildRegister {
        operation_id: id(), child_id: "native-child".into(),
        child_session_id: "native-child-conversation".into(), parent_child_id: None,
    };
    for fence in 0..4 {
        let mut forged = registering.clone();
        let caller = caller_mut(&mut forged);
        match fence {
            0 => caller.process.as_mut().unwrap().start_ticks += 1,
            1 => caller.actual_agent_kind = Some(NativeAgentKind::Other("shell".into())),
            2 => caller.main_omp_session_id = Some("not-bound-main".into()),
            3 => caller.subagent_id = Some("different-child".into()),
            _ => unreachable!(),
        }
        let before = fixture.state_bytes();
        assert_code(fixture.apply(&forged, action.clone()), "child_identity_unproven");
        assert_eq!(fixture.state_bytes(), before);
    }
    let mut receiver = fixture.service.subscribe();
    fixture.apply(&registering, action.clone()).unwrap();
    let credentials = fixture.state().credentials.into_iter().filter(|record|
        record.run_id == worker && record.role == JobRole::Child
            && record.subject_child_id.as_deref() == Some("native-child")).collect::<Vec<_>>();
    assert_eq!(credentials.len(), 1);
    let Actor::Agent(main) = &parent else { unreachable!() };
    let path = fixture.service.job_child_credential_file(main, "native-child").unwrap();
    let child_auth = credentials::read_job_auth(&path).unwrap();
    let credential_bytes = std::fs::read(&path).unwrap();
    assert_eq!(child_auth.credential_id, credentials[0].credential_id);
    let before = fixture.state_bytes();
    let revision = fixture.state().revision;
    assert!(receiver.has_changed().unwrap(), "The first registration must publish its commit");
    assert_eq!(*receiver.borrow_and_update(), revision);
    fixture.apply(&registering, action.clone()).unwrap();
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().revision, revision);
    assert_eq!(std::fs::read(&path).unwrap(), credential_bytes, "Replay cannot remint the child token");
    assert!(!receiver.has_changed().unwrap());
    for field in 0..3 {
        let mut changed = action.clone();
        let OrchestrationAction::JobChildRegister { child_id, child_session_id, parent_child_id, .. } = &mut changed else { unreachable!() };
        match field {
            0 => *child_id = "changed-child".into(),
            1 => *child_session_id = "changed-child-session".into(),
            2 => *parent_child_id = Some("changed-parent".into()),
            _ => unreachable!(),
        }
        assert_code(fixture.apply(&registering, changed), "operation_conflict");
        assert_eq!(fixture.state_bytes(), before);
    }
    let caller = caller_mut(&mut registering);
    caller.auth = child_auth;
    assert_eq!(fixture.service.job_child_credential_file(caller, "native-child").unwrap_err().code,
        "actor_forbidden");
    assert_code(fixture.apply(&registering, action), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn run_attach_never_replaces_physical_process_with_a_bearer_credential() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let Actor::Agent(main) = &actor else { unreachable!() };
    let original = fixture.run(&worker);
    let action = OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: main.auth.attachment_id.clone(),
        expected_attachment_epoch: main.auth.attachment_epoch,
        main_session_id: main.omp_session_id.clone().unwrap(),
    };
    for fence in 0..4 {
        let mut forged = actor.clone();
        let caller = caller_mut(&mut forged);
        match fence {
            0 => caller.process.as_mut().unwrap().start_ticks += 1,
            1 => caller.process = None,
            2 => caller.actual_agent_kind = Some(NativeAgentKind::Other("shell".into())),
            3 => caller.omp_session_id = Some("other-conversation".into()),
            _ => unreachable!(),
        }
        let before = fixture.state_bytes();
        assert_code(fixture.apply(&forged, action.clone()), "attachment_conflict");
        assert_eq!(fixture.state_bytes(), before);
        assert_eq!(fixture.run(&worker).attachment.as_ref().unwrap().epoch, original.attachment.as_ref().unwrap().epoch);
        assert_eq!(fixture.run(&worker).checkout_claim_id, original.checkout_claim_id);
    }
}

#[test]
fn reuse_start_prefers_selected_matching_root_then_stable_latest_open_root() {
    let fixture = Fixture::new();
    let first = fixture.root();
    let second = fixture.root();
    for root in [&first, &second] {
        fixture.seed_run(root, |run| run.updated_at = "2026-10-10T00:00:00Z".into());
    }
    let state = fixture.state();
    let latest = preferred_project_root(&state.runs, SESSION, &fixture.project().key, None).unwrap();
    let preferred = state.runs[latest].run_id.clone();
    // Equal timestamps have a deterministic UUID lexical preference.
    assert_eq!(preferred.as_str(), first.as_str().min(second.as_str()));
    let other = if preferred == first { second.clone() } else { first.clone() };
    let selected = fixture.operator(OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
        selected_root_id: Some(other.clone()), expected_project_key: Some(fixture.project().key),
    });
    let OrchestrationActionResult::SupervisorStarted { start } = selected.result else { unreachable!() };
    assert_eq!(start.root_id, other);
    assert_eq!(fixture.state().runs.len(), 2);
    fixture.seed_run(&preferred, |run| run.updated_at = "2026-10-11T00:00:00Z".into());
    let latest = fixture.operator(OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
        selected_root_id: Some(id()), expected_project_key: Some(fixture.project().key),
    });
    let OrchestrationActionResult::SupervisorStarted { start } = latest.result else { unreachable!() };
    assert_eq!(start.root_id, preferred);
    fixture.operator(OrchestrationAction::CancelRun { run_id: preferred.clone() });
    let open = fixture.operator(OrchestrationAction::SupervisorStart {
        operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
        selected_root_id: Some(preferred), expected_project_key: Some(fixture.project().key),
    });
    let OrchestrationActionResult::SupervisorStarted { start } = open.result else { unreachable!() };
    assert_eq!(start.root_id, other, "Closed history is not implicitly reopened");
    assert_eq!(fixture.state().runs.len(), 2);
}

#[test]
fn obsolete_schema_reads_fresh_current_state_and_replaces_records_only_on_ordinary_save() {
    let fixture = Fixture::new();
    let source = serde_json::to_vec(&serde_json::json!({
        "schema": 1, "revision": 17, "runs": [{"obsolete": "permission-era state"}],
        "messages": [], "subagents": [], "task_intents": [], "assignment_intents": [],
    })).unwrap();
    {
        let locked = fixture.service.store.lock().unwrap();
        locked.state_dir().write("state.json", &source).unwrap();
    }
    let service = OrchestrationService::open(&fixture.configuration).unwrap();
    let state = service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.schema, store::SCHEMA);
    assert_eq!(state.revision, 17);
    assert!(state.runs.is_empty());
    assert!(state.operations.is_empty());
    assert!(state.credentials.is_empty() && state.checkout_claims.is_empty());
    assert_eq!(fixture.state_bytes(), source, "Read-only open does not rewrite obsolete state");
    let response = service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: Some(17),
            action: OrchestrationAction::SupervisorStart {
                operation_id: id(), mode: SupervisorStartMode::Another, target: Some(target()),
                label: None, selected_root_id: None, expected_project_key: Some(fixture.project().key),
            },
        }, &ResolvedMutationInput { project: Some(fixture.project()), ..ResolvedMutationInput::default() }).unwrap();
    assert_eq!(response.revision, 18);
    let state = service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.schema, store::SCHEMA);
    assert_eq!(state.runs.len(), 1);
    assert_eq!(state.operations.len(), 1);
    assert_ne!(fixture.state_bytes(), source);
}

#[test]
fn runtime_composition_does_not_hide_malformed_or_unsupported_store_errors() {
    for source in [b"not-json".as_slice(), br#"{"schema":99,"revision":17}"#.as_slice()] {
        let fixture = Fixture::new();
        {
            let locked = fixture.service.store.lock().unwrap();
            locked.state_dir().write("state.json", source).unwrap();
        }
        match OrchestrationService::open(&fixture.configuration) {
            Err(error) => assert_eq!(error.code, "state_corrupt"),
            Ok(_) => panic!("Malformed or unsupported state must not open"),
        }
        assert_eq!(fixture.state_bytes(), source);
    }
}

#[test]
fn physical_resume_preserves_current_writer_but_needs_an_authenticated_extension() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, mut actor) = fixture.working(&root);
    let original = fixture.run(&worker);
    let observed = original.attachment.clone().unwrap();
    let claim = fixture.state().checkout_claims.into_iter()
        .find(|claim| original.checkout_claim_id.as_ref() == Some(&claim.claim_id)).unwrap();
    fixture.seed_state(|state| {
        state.credentials.retain(|record| record.run_id != worker);
        state.operations.retain(|record| record.kind != store::DurableOperationKind::RunAttach
            || record.actor_run_id.as_deref() != Some(worker.as_str()));
    });
    assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
    for staged in [false, true] {
        if staged {
            caller_mut(&mut actor).auth = fixture.auth(&worker, JobRole::Worker, None);
        }
        let response = fixture.resume_observed(&worker, observed.clone());
        assert_eq!(response.resume.phase, ResumePhase::Blocked);
        assert_eq!(response.resume.cause.as_ref().unwrap().code, "current_extension_unavailable");
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
            "Staged credential alone is not evidence that this living OMP loaded the extension");
    }
    // Ordinary durable authority stays available while semantic connection is absent.
    fixture.apply(&actor, report(ReportKind::Progress, None, None)).unwrap();
    assert_eq!(fixture.run(&worker).attachment.as_ref().unwrap().state, AttachmentState::Disconnected);
    let blocked = fixture.run(&worker).resume.unwrap();
    let Actor::Agent(caller) = &actor else { unreachable!() };
    fixture.apply(&actor, OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    }).unwrap();
    let connected = fixture.run(&worker);
    assert_eq!(connected.attachment.as_ref().unwrap().state, AttachmentState::Connected);
    assert_eq!(connected.attachment.as_ref().unwrap().epoch, observed.epoch);
    assert_eq!(connected.attachment.as_ref().unwrap().process, observed.process);
    assert_eq!(connected.checkout_claim_id, original.checkout_claim_id);
    assert_eq!(serde_json::to_value(&connected.dispatch).unwrap(), serde_json::to_value(&original.dispatch).unwrap());
    let resumed = connected.resume.as_ref().unwrap();
    assert_eq!(resumed.operation_id, blocked.operation_id);
    assert_eq!(resumed.phase, ResumePhase::Reattached);
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
    let replay = fixture.operator(OrchestrationAction::RunResume {
        run_id: worker.clone(), operation_id: blocked.operation_id,
        expected_attachment_epoch: Some(observed.epoch),
        mode: ResumeMode::ReattachOrRestart, decision_id: None,
    });
    let OrchestrationActionResult::RunResumed { resume: replay } = replay.result else { unreachable!() };
    assert!(replay.duplicate);
    assert_eq!(replay.resume.phase, ResumePhase::Reattached);
    assert!(replay.resume.cause.is_none());
    assert_eq!(fixture.state_bytes(), before, "Owner replay returns the immediately recovered receipt without relaunch");
}

#[test]
fn physical_start_observation_without_current_extension_binding_does_not_reuse_a_root() {
    for staged in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        fixture.launch_and_bind(&root);
        let original = fixture.run(&root);
        let observed = original.attachment.clone().unwrap();
        fixture.seed_state(|state| {
            state.credentials.retain(|record| record.run_id != root);
            state.operations.retain(|record| record.kind != store::DurableOperationKind::RunAttach
                || record.actor_run_id.as_deref() != Some(root.as_str()));
        });
        if staged {
            fixture.auth(&root, JobRole::Root, None);
        }
        let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            OrchestrationMutationRequest {
                session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
                action: OrchestrationAction::SupervisorStart {
                    operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
                    selected_root_id: Some(root.clone()), expected_project_key: Some(fixture.project().key),
                },
            }, &ResolvedMutationInput {
                project: Some(fixture.project()), attachment_observation: Some(observed.clone()),
                ..ResolvedMutationInput::default()
            }).unwrap();
        let OrchestrationActionResult::SupervisorStarted { start } = response.result else { unreachable!() };
        assert_eq!(start.root_id, root);
        assert_eq!(start.outcome, SupervisorStartOutcome::Resuming);
        let current = fixture.run(&root);
        assert_eq!(current.resume.as_ref().unwrap().phase, ResumePhase::Blocked);
        assert_eq!(current.resume.as_ref().unwrap().cause.as_ref().unwrap().code, "current_extension_unavailable");
        assert_eq!(current.resume.as_ref().unwrap().cause.as_ref().unwrap().class, ResumeCauseClass::Recover);
        let attachment = current.attachment.as_ref().unwrap();
        assert_eq!(attachment.state, AttachmentState::Disconnected);
        assert_eq!(attachment.epoch, observed.epoch);
        assert_eq!(attachment.process, observed.process);
        assert_eq!(serde_json::to_value(current.dispatch).unwrap(), serde_json::to_value(original.dispatch).unwrap());
        assert_eq!(fixture.state().runs.len(), 1);
        assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&root));
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
fn revoked_credentials_or_stale_attach_receipts_never_prove_current_extension_binding() {
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
                for operation in state.operations.iter_mut().filter(|record|
                    record.kind == store::DurableOperationKind::RunAttach
                        && record.actor_run_id.as_deref() == Some(worker.as_str())) {
                    if let Some(OrchestrationActionResult::RunAttached { attach }) = &mut operation.result {
                        attach.attachment.epoch += 1;
                    }
                }
            }
        });
        assert!(!fixture.service.recorded_runs_with_bindings(SESSION).unwrap().2.contains_key(&worker));
        let result = fixture.resume_observed(&worker, observed.clone());
        assert_eq!(result.resume.phase, ResumePhase::Blocked);
        assert_eq!(result.resume.cause.as_ref().unwrap().code, "current_extension_unavailable");
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
    let Actor::Agent(caller) = &actor else { unreachable!() };
    fixture.apply(&actor, OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    }).unwrap();
    let observed = fixture.run(&root).attachment.unwrap();
    let response = fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: Some(fixture.state().revision),
            action: OrchestrationAction::SupervisorStart {
                operation_id: id(), mode: SupervisorStartMode::ReuseOrResume, target: Some(target()), label: None,
                selected_root_id: Some(root.clone()), expected_project_key: Some(fixture.project().key),
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
fn closed_history_with_a_recorded_physical_process_cannot_be_adopted_as_another_root() {
    let fixture = Fixture::new();
    let seed = fixture.root();
    let actor = fixture.launch_tab(&seed);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let bootstrap = BootstrapCaller {
        endpoint_identity: caller.endpoint_identity.clone(), session_id: caller.session_id.clone(),
        workspace_id: caller.workspace_id.clone(), tab_id: "fresh-adoption-tab".into(),
        pane_id: "fresh-adoption-pane".into(), boot_id: caller.boot_id.clone(),
        terminal_id: Some("fresh-adoption-terminal".into()),
        native_session_id: Some("fresh-adoption-native".into()), omp_session_id: Some("fresh-adoption-native".into()),
        agent_kind: Some(AgentKind::Main), actual_agent_kind: caller.actual_agent_kind.clone(),
        process: Some(live_process()),
    };
    let adopted = run_result(fixture.apply(&Actor::Bootstrap(bootstrap.clone()),
        OrchestrationAction::RunAdopt { label: "Fresh unbound process".into() }).unwrap()).0;
    assert_eq!(fixture.run(&adopted).kind, RunKind::Adopted);
    assert_eq!(fixture.run(&adopted).attachment.as_ref().unwrap().process, bootstrap.process);
    fixture.seed_run(&adopted, |run| {
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Cancelled);
    });
    let before = fixture.state_bytes();
    let state = fixture.state();
    assert_eq!(fixture.run(&adopted).attachment.as_ref().unwrap().process, bootstrap.process,
        "Closing tracking is not proof that its physical process exited");
    assert_code(fixture.apply(&Actor::Bootstrap(bootstrap),
        OrchestrationAction::RunAdopt { label: "Must not duplicate the retained incarnation".into() }), "caller_mismatch");
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(fixture.state().runs.len(), state.runs.len());
    assert_eq!(fixture.state().credentials.len(), state.credentials.len());
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
    let join = state.operations.iter().find(|record| &record.operation_id == joined_id).unwrap();
    assert_eq!(join.run_id.as_deref(), Some(worker.as_str()));
    assert_eq!(join.resume_operation_id.as_deref(), Some(first.resume.operation_id.as_str()));
    assert!(join.result.is_none());
    assert!(join.effect_receipts.is_empty());
    fixture.launch_and_bind(&worker);
    assert_eq!(fixture.run(&worker).resume.as_ref().unwrap().phase, ResumePhase::Connected);
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
fn worker_can_resume_own_job_but_child_cannot_and_replace_never_stops_a_same_boot_writer() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (_, worker, actor) = fixture.working(&root);
    let child = fixture.child_actor(&actor, &worker, "resume-child");
    let bytes = fixture.state_bytes();
    assert_code(fixture.apply(&child, fixture.resume_action(&worker)), "actor_forbidden");
    assert_eq!(fixture.state_bytes(), bytes);
    let action = fixture.resume_action(&worker);
    let response = fixture.apply(&actor, action).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
    assert_eq!(resume.resume.phase, ResumePhase::Blocked);
    // A decision is specific to this observed protected incarnation.
    let response = fixture.service.advance_resume_observed(&worker, &resume.resume.operation_id,
        &ResolvedMutationInput {
            observation_cause: Some(ResumeCause {
                code: "living_writer".into(), message: "Current writer is alive".into(),
                class: ResumeCauseClass::Decision,
            }), ..ResolvedMutationInput::default()
        }).unwrap();
    let original = fixture.run(&worker);
    let decision = response.resume.decision_id.unwrap();
    let response = fixture.apply(&actor, OrchestrationAction::RunResume {
        run_id: worker.clone(), operation_id: id(),
        expected_attachment_epoch: original.attachment.as_ref().map(|attachment| attachment.epoch),
        mode: ResumeMode::ReplaceExisting, decision_id: Some(decision),
    }).unwrap();
    let OrchestrationActionResult::RunResumed { resume } = response.result else { unreachable!() };
    assert_eq!(resume.resume.phase, ResumePhase::Blocked);
    assert_eq!(resume.resume.cause.as_ref().unwrap().code, "same_boot_replacement_unsupported");
    let current = fixture.run(&worker);
    assert_review_fields_only(&original, &current);
    assert!(current.retirement.is_none());
    assert_ne!(current.attachment.as_ref().unwrap().state, AttachmentState::Quiescing);
    let state = fixture.state();
    let operation = state.operations.iter().find(|record| record.operation_id == resume.resume.operation_id).unwrap();
    assert!(operation.result.is_none());
    assert!(operation.effect_receipts.is_empty());
    assert!(state.checkout_claims.iter().all(|claim| claim.state != claims::ClaimState::Released));
    assert!(state.credentials.iter().filter(|credential| credential.run_id == worker)
        .all(|credential| credential.state == credentials::CredentialState::Active));
}

#[test]
fn private_attachment_after_native_launch_proof_starts_once_and_sends_one_brief() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Late private attachment");
    let worker = fixture.start(&root, &task);
    fixture.setup(&worker);
    let actor = fixture.launch_tab(&worker);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    // Native runtime proof is physical evidence, not an applied extension attach.
    fixture.seed_run(&worker, |run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.main_session_id = caller.omp_session_id.clone();
        attachment.process = caller.process.clone();
    });
    fixture.service.record_launch_verified(&fixture.run(&worker)).unwrap();
    assert_eq!(fixture.run(&worker).stage, RunStage::Starting);
    assert!(fixture.state().messages.iter().all(|message|
        message.to_run_id != worker || message.kind != MessageKind::WorkBrief));
    let attach = OrchestrationAction::RunAttach {
        operation_id: id(), attachment_id: caller.auth.attachment_id.clone(),
        expected_attachment_epoch: caller.auth.attachment_epoch,
        main_session_id: caller.omp_session_id.clone().unwrap(),
    };
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
