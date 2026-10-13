use super::*;
use std::path::PathBuf;

fn worker() -> Run {
    Run {
        session_id: "cleanup-test-session".into(), brief: String::new(), run_id: id(),
        kind: RunKind::Worker, label: "worker".into(), root_id: id(), parent_run_id: Some(id()),
        task_id: None, attempt: 1, task_revision_at_start: None, stage: RunStage::Closed,
        assignment_generation: None, launch_config: Default::default(),
        close_reason: Some(CloseReason::Accepted), dispatch: None, target: None, setup: None,
        cleanup_policy: Some(WorkerCleanupPolicy::OwnedWhenSafe), last_report: None, result: None,
        source_output: None, integration: Integration::Pending, annotations: Vec::new(), project: None,
        checkout: None, attachment: None, checkout_claim_id: None, resume: None, retirement: None,
        created_at: now(), updated_at: now(),
    }
}

fn space() -> OwnedResourceIdentity {
    OwnedResourceIdentity::Space { identity: OwnedSpaceIdentity {
        creation_operation_id: id(), endpoint_identity: "endpoint".into(),
        session_id: "cleanup-test-session".into(), workspace_id: id(), checkout_path: "/owned/checkout".into(),
    } }
}

struct Fixture { root: PathBuf, service: OrchestrationService, run: Run, resource: OwnedResourceRecord }
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-cleanup-{}", id()));
        let store = OrchestrationStore::open(&root).unwrap();
        let (revision, _) = watch::channel(0);
        let service = OrchestrationService { store, revision, launch_config: Default::default() };
        let run = worker();
        let resource = {
            let locked = service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(run.clone());
            let index = insert_resource(&mut state.owned_resources, &run, space()).unwrap();
            let resource = state.owned_resources[index].clone();
            locked.save(&mut state).unwrap();
            resource
        };
        Self { root, service, run, resource }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

#[test]
fn creation_identity_deduplicates_only_exact_provenance() {
    let run = worker();
    let identity = space();
    let mut state = OrchestrationState::default();
    let first = insert_resource(&mut state.owned_resources, &run, identity.clone()).unwrap();
    assert_eq!(insert_resource(&mut state.owned_resources, &run, identity.clone()).unwrap(), first);
    assert_eq!(state.owned_resources.len(), 1);
    let mut contradictory = identity;
    let OwnedResourceIdentity::Space { identity } = &mut contradictory else { unreachable!() };
    identity.creation_operation_id = id();
    assert!(insert_resource(&mut state.owned_resources, &run, contradictory).is_err());
    assert_eq!(state.owned_resources.len(), 1);
}

#[test]
fn intent_is_durable_before_dispatch_and_uncertain_effect_is_not_repeated() {
    let fixture = Fixture::new();
    let intent = fixture.service.begin_cleanup_effect(&fixture.resource, &fixture.run).unwrap().unwrap();
    let reopened = OrchestrationStore::open(&fixture.root).unwrap();
    let saved = reopened.lock().unwrap().read().unwrap();
    assert_eq!(saved.owned_resources[0].cleanup.state, CleanupState::Running);
    assert_eq!(saved.owned_resources[0].cleanup.effects[0].state, CleanupEffectState::Intent);
    assert!(fixture.service.begin_cleanup_effect(&intent, &fixture.run).unwrap().is_none());
    fixture.service.save_evaluation(&intent, CleanupEvaluation::Absent).unwrap();
    let saved = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(saved.owned_resources[0].cleanup.state, CleanupState::Removed);
    assert_eq!(saved.owned_resources[0].cleanup.effects[0].state, CleanupEffectState::Absent);
}

#[test]
fn missing_space_receipt_does_not_veto_an_independent_git_wait() {
    let fixture = Fixture::new();
    let mut second = fixture.resource.clone();
    second.identity = OwnedResourceIdentity::Branch { identity: OwnedGitIdentity {
        creation_operation_id: id(), machine_id: "machine".into(), repository_key: "repository".into(),
        repository_root: "/retained/repository".into(), repository_device: "1".into(), repository_inode: "2".into(),
        common_dir: "/retained/repository/.git".into(), common_device: "1".into(), common_inode: "3".into(),
        checkout_path: "/owned/checkout".into(), checkout_device: "1".into(), checkout_inode: "4".into(),
        checkout_key: "1:4".into(), branch_ref: Some("refs/heads/owned".into()), creation_tip: "a".repeat(40),
    } };
    second.resource_id = identity_digest(&second.identity).unwrap();
    second.creation_operation_id = Some(creation_operation(&second.identity).into());
    second.kind = OwnedResourceKind::Branch;
    second.cleanup.state = CleanupState::WaitingIntegration;
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.owned_resources.push(second.clone());
        locked.save(&mut state).unwrap();
    }
    fixture.service.save_evaluation(&fixture.resource, CleanupEvaluation::Absent).unwrap();
    let saved = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(saved.owned_resources[0].cleanup.state, CleanupState::Removed);
    assert_eq!(saved.owned_resources[1], second);
}

#[test]
fn summary_redacts_effect_output_and_reason_text_without_losing_states() {
    let run = worker();
    let mut state = OrchestrationState::default();
    let index = insert_resource(&mut state.owned_resources, &run, space()).unwrap();
    state.runs.push(run.clone());
    let resource = &mut state.owned_resources[index];
    resource.cleanup.state = CleanupState::Unknown;
    resource.cleanup.reason = Some(reason(CleanupReasonClass::Authority, "creation_identity_unavailable", "/private/path diagnostic"));
    resource.cleanup.effects.push(CleanupEffectReceipt { effect_id: id(), kind: resource.kind,
        identity_digest: resource.resource_id.clone(), state: CleanupEffectState::Unknown,
        evidence: Some("/private/process/evidence".into()), updated_at: now() });
    let summary = summaries(&state, &run.session_id);
    assert_eq!(summary[0].unknown_count, 1);
    assert!(summary[0].items[0].cleanup.effects[0].evidence.is_none());
    assert!(!summary[0].items[0].cleanup.reason.as_ref().unwrap().message.contains("/private"));
    assert_eq!(state.owned_resources[index].cleanup.effects[0].evidence.as_deref(), Some("/private/process/evidence"));
}

#[test]
fn borrowed_and_unknown_resources_never_receive_cleanup_effect_authority() {
    let fixture = Fixture::new();
    for ownership in [ResourceOwnership::Borrowed, ResourceOwnership::Unknown] {
        let expected = {
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.owned_resources[0].ownership = ownership;
            state.owned_resources[0].authority = CleanupAuthority::AuthorityUnproven;
            let expected = state.owned_resources[0].clone();
            locked.save(&mut state).unwrap();
            expected
        };
        assert!(fixture.service.begin_cleanup_effect(&expected, &fixture.run).unwrap().is_none());
    }
}

#[test]
fn aggregate_counts_keep_waits_unknowns_and_borrowed_resources_independent() {
    assert_eq!(counts([CleanupState::Removed, CleanupState::Kept, CleanupState::Unknown,
        CleanupState::WaitingIntegration, CleanupState::Reconciling, CleanupState::Pending].into_iter()),
        (1, 3, 1, 1));
}

fn terminal_identity() -> RetirementIdentity {
    RetirementIdentity {
        attachment_id: Some("attachment".into()), attachment_epoch: Some(1),
        launch_mode: Some(AttachmentLaunchMode::ShellAgent), run_attempt: 1, launch_attempt: 2,
        launch_tag: "launch".into(), endpoint_identity: "endpoint".into(),
        session_id: "cleanup-test-session".into(), workspace_id: "space".into(),
        tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: "terminal".into(),
        herdr_boot_id: Some("herdr-boot".into()), conversation: None,
        process: Some(NativeProcessIdentity { pid: 101, start_ticks: 102, kernel_boot_id: Some("kernel".into()) }),
        shell: Some(NativeShellIdentity {
            process: NativeProcessIdentity { pid: 201, start_ticks: 202, kernel_boot_id: Some("kernel".into()) },
            executable_device: "device".into(), executable_inode: "inode".into(), argv_digest: "argv".into(),
        }),
    }
}

fn terminal_run(identity: RetirementIdentity, state: RetirementState) -> Run {
    let mut run = worker();
    run.retirement = Some(RunRetirement {
        retirement_id: "retirement-effect".into(), trigger: RetirementTrigger::Accept,
        result_message_id: "accepted-result".into(), task_revision: "accepted-revision".into(),
        identity: Some(identity), state, created_at: "created".into(), updated_at: "updated".into(),
    });
    run
}

#[test]
fn late_conversation_evaluates_saved_unknown_without_rewriting_creation_receipts() {
    for (state, expected) in [
        (RetirementState::Retained { at: now(), reason: RetainReason::NativeProcessUnverifiable, stopped: false }, CleanupState::Kept),
        (RetirementState::Retained { at: now(), reason: RetainReason::NativeProcessUnverifiable, stopped: true }, CleanupState::Kept),
        (RetirementState::Waiting { blockers: Vec::new() }, CleanupState::Waiting),
        (RetirementState::CloseIntent { at: now() }, CleanupState::Waiting),
        (RetirementState::Unknown { at: now(), phase: RetirementPhase::TerminalClose, detail: "uncertain".into() }, CleanupState::Unknown),
        (RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent }, CleanupState::Removed),
        (RetirementState::Retired { at: now(), terminal: TerminalOutcome::ClosedByCockpit }, CleanupState::Removed),
        (RetirementState::Retired { at: now(), terminal: TerminalOutcome::AbsentAfterUncertainClose }, CleanupState::Removed),
    ] {
        let fixture = Fixture::new();
        let creation = terminal_identity();
        let creation_bytes = serde_json::to_vec(&creation).unwrap();
        let mut accepted = creation.clone();
        accepted.conversation = Some(ConversationIdentity { harness: "omp".into(), session_id: "late-chat".into() });
        let run = terminal_run(accepted, state);
        let resource = {
            let locked = fixture.service.store.lock().unwrap();
            let mut stored = locked.read().unwrap();
            let index = insert_resource(&mut stored.owned_resources, &run,
                OwnedResourceIdentity::WorkerTerminal { identity: creation }).unwrap();
            let resource = &mut stored.owned_resources[index];
            resource.cleanup.state = CleanupState::Unknown;
            resource.cleanup.reason = Some(reason(CleanupReasonClass::Authority, "terminal_identity_changed", "Old metadata mismatch"));
            let resource = resource.clone();
            stored.runs.push(run.clone());
            locked.save(&mut stored).unwrap();
            resource
        };
        let mut before = fixture.service.store.lock().unwrap().read().unwrap();
        fixture.service.save_evaluation(&resource, terminal_evaluation(&resource, &run)).unwrap();
        let after = fixture.service.store.lock().unwrap().read().unwrap();
        let evaluated = &after.owned_resources[1];
        assert_eq!(evaluated.cleanup.state, expected);
        assert_eq!(evaluated.resource_id, resource.resource_id);
        let OwnedResourceIdentity::WorkerTerminal { identity } = &evaluated.identity else { unreachable!() };
        assert_eq!(serde_json::to_vec(identity).unwrap(), creation_bytes);
        assert_ne!(evaluated.cleanup.reason.as_ref().map(|why| why.code.as_str()), Some("terminal_identity_changed"));
        before.revision = after.revision;
        before.owned_resources[1].cleanup = evaluated.cleanup.clone();
        assert_eq!(serde_json::to_value(before).unwrap(), serde_json::to_value(&after).unwrap());
        fixture.service.save_evaluation(evaluated, terminal_evaluation(evaluated, &run)).unwrap();
        assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(),
            serde_json::to_value(after).unwrap(), "same evaluation must not create another revision");
    }
}

#[test]
fn late_conversation_receipt_sync_keeps_one_original_effect_identity() {
    let creation = terminal_identity();
    let mut accepted = creation.clone();
    accepted.conversation = Some(ConversationIdentity { harness: "omp".into(), session_id: "late-chat".into() });
    let run = terminal_run(accepted, RetirementState::CloseIntent { at: "intent".into() });
    let mut state = OrchestrationState::default();
    insert_resource(&mut state.owned_resources, &run, OwnedResourceIdentity::WorkerTerminal { identity: creation }).unwrap();
    let unrelated = insert_resource(&mut state.owned_resources, &run, space()).unwrap();
    let unrelated = state.owned_resources[unrelated].clone();
    let original = state.owned_resources[0].clone();
    assert_eq!(insert_resource(&mut state.owned_resources, &run, original.identity.clone()).unwrap(), 0);
    assert!(insert_resource(&mut state.owned_resources, &run, OwnedResourceIdentity::WorkerTerminal {
        identity: run.retirement.as_ref().unwrap().identity.clone().unwrap(),
    }).is_err(), "receipt joining must not relax creation provenance deduplication");
    assert_eq!(state.owned_resources[0], original);
    state.runs.push(run);
    for (retirement_state, effect_state) in [
        (RetirementState::CloseIntent { at: "intent".into() }, CleanupEffectState::Intent),
        (RetirementState::Unknown { at: "unknown".into(), phase: RetirementPhase::TerminalClose, detail: "unconfirmed".into() }, CleanupEffectState::Unknown),
        (RetirementState::Retired { at: "absent".into(), terminal: TerminalOutcome::AbsentAfterUncertainClose }, CleanupEffectState::Absent),
        (RetirementState::Retired { at: "closed".into(), terminal: TerminalOutcome::ClosedByCockpit }, CleanupEffectState::Applied),
    ] {
        state.runs[0].retirement.as_mut().unwrap().state = retirement_state;
        sync_retirement_receipts(&mut state, 0);
        let mut expected = original.clone();
        expected.cleanup.effects.push(CleanupEffectReceipt {
            effect_id: "retirement-effect".into(), kind: OwnedResourceKind::WorkerTerminal,
            identity_digest: original.resource_id.clone(), state: effect_state,
            evidence: None, updated_at: "updated".into(),
        });
        assert_eq!(state.owned_resources[0], expected);
        assert_eq!(state.owned_resources[1], unrelated);
        let before = serde_json::to_value(&state).unwrap();
        sync_retirement_receipts(&mut state, 0);
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
}

#[test]
fn receipt_joins_refuse_every_physical_difference_and_conflicting_conversation() {
    let creation = terminal_identity();
    let mut accepted = creation.clone();
    accepted.conversation = Some(ConversationIdentity { harness: "omp".into(), session_id: "late-chat".into() });
    let changes: &[(&str, fn(&mut RetirementIdentity))] = &[
        ("attachment ID", |i| i.attachment_id = Some("other".into())),
        ("attachment absent", |i| i.attachment_id = None),
        ("attachment epoch", |i| i.attachment_epoch = Some(2)),
        ("epoch absent", |i| i.attachment_epoch = None),
        ("launch mode", |i| i.launch_mode = Some(AttachmentLaunchMode::DirectProcess)),
        ("launch mode absent", |i| i.launch_mode = None),
        ("run attempt", |i| i.run_attempt += 1),
        ("launch attempt", |i| i.launch_attempt += 1),
        ("launch tag", |i| i.launch_tag = "other".into()),
        ("endpoint", |i| i.endpoint_identity = "other".into()),
        ("session", |i| i.session_id = "other".into()),
        ("Space", |i| i.workspace_id = "other".into()),
        ("tab", |i| i.tab_id = "other".into()),
        ("pane", |i| i.pane_id = "other".into()),
        ("terminal", |i| i.terminal_id = "other".into()),
        ("Herdr boot", |i| i.herdr_boot_id = Some("other".into())),
        ("Herdr boot absent", |i| i.herdr_boot_id = None),
        ("process absent", |i| i.process = None),
        ("process PID", |i| i.process.as_mut().unwrap().pid += 1),
        ("process start", |i| i.process.as_mut().unwrap().start_ticks += 1),
        ("process boot", |i| i.process.as_mut().unwrap().kernel_boot_id = Some("other".into())),
        ("process boot absent", |i| i.process.as_mut().unwrap().kernel_boot_id = None),
        ("shell absent", |i| i.shell = None),
        ("shell PID", |i| i.shell.as_mut().unwrap().process.pid += 1),
        ("shell start", |i| i.shell.as_mut().unwrap().process.start_ticks += 1),
        ("shell boot", |i| i.shell.as_mut().unwrap().process.kernel_boot_id = Some("other".into())),
        ("shell boot absent", |i| i.shell.as_mut().unwrap().process.kernel_boot_id = None),
        ("shell executable device", |i| i.shell.as_mut().unwrap().executable_device = "other".into()),
        ("shell executable inode", |i| i.shell.as_mut().unwrap().executable_inode = "other".into()),
        ("shell argv", |i| i.shell.as_mut().unwrap().argv_digest = "other".into()),
    ];
    let mut cases = Vec::new();
    for (label, change) in changes {
        let mut changed = accepted.clone();
        change(&mut changed);
        cases.push((*label, creation.clone(), Some(changed)));
    }
    for field in ["harness", "session"] {
        let mut changed = accepted.clone();
        let conversation = changed.conversation.as_mut().unwrap();
        if field == "harness" { conversation.harness = "other".into(); }
        else { conversation.session_id = "other".into(); }
        cases.push((field, accepted.clone(), Some(changed)));
    }
    cases.push(("retirement identity absent", creation, None));
    for (label, creation, changed) in cases {
        if let Some(changed) = &changed {
            assert!(!retirement::receipt_identity_matches(&creation, changed), "{label}");
            assert!(!retirement::receipt_identity_matches(changed, &creation), "{label}");
        }
        let mut run = terminal_run(accepted.clone(),
            RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent });
        run.retirement.as_mut().unwrap().identity = changed;
        let mut state = OrchestrationState::default();
        insert_resource(&mut state.owned_resources, &run, OwnedResourceIdentity::WorkerTerminal { identity: creation }).unwrap();
        state.runs.push(run);
        let before = serde_json::to_value(&state).unwrap();
        sync_retirement_receipts(&mut state, 0);
        assert_eq!(serde_json::to_value(&state).unwrap(), before, "{label}: no receipt join");
        assert!(matches!(terminal_evaluation(&state.owned_resources[0], &state.runs[0]),
            CleanupEvaluation::Unknown(why) if why.code == "terminal_identity_changed"), "{label}");
    }
}

#[test]
fn retired_pane_never_releases_claim_or_authorizes_checkout_resource_effects() {
    for terminal in [TerminalOutcome::ClosedByCockpit, TerminalOutcome::AlreadyAbsent] {
        let fixture = Fixture::new();
        let (_, checkout) = project_identity::resolve_owned_directory(&fixture.root).unwrap();
        let mut identity = terminal_identity();
        identity.attachment_id = Some(id());
        identity.process.as_mut().unwrap().kernel_boot_id = crate::process_identity::kernel_boot_id();
        identity.shell.as_mut().unwrap().process.kernel_boot_id = crate::process_identity::kernel_boot_id();
        let mut run = fixture.run.clone();
        run.attachment = Some(ExecutionAttachment {
            attachment_id: identity.attachment_id.clone().unwrap(), epoch: identity.attachment_epoch.unwrap(),
            state: AttachmentState::Connected, launch_mode: identity.launch_mode,
            location: Some(RunLocation {
                boot_id: identity.herdr_boot_id.clone(), terminal_id: Some(identity.terminal_id.clone()),
                native_session_id: None, endpoint_identity: identity.endpoint_identity.clone(),
                session_id: identity.session_id.clone(), workspace_id: identity.workspace_id.clone(),
                tab_id: identity.tab_id.clone(), pane_id: identity.pane_id.clone(), launch_tag: identity.launch_tag.clone(),
            }),
            conversation: Some(ConversationIdentity { harness: "omp".into(), session_id: "accepted-chat".into() }),
            process: identity.process.clone(), shell: identity.shell.clone(),
            machine_id: Some(checkout.machine_id.clone()), launch_attempt: identity.launch_attempt, updated_at: now(),
        });
        run.source_output = Some(SourceOutput::NonCode {
            result_message_id: "accepted-result".into(), task_revision: "accepted-revision".into(),
            artifacts: Vec::new(), focused_checks: Vec::new(),
        });
        run.integration = Integration::NotApplicable {
            reason: "Accepted non-code result".into(), result_message_id: "accepted-result".into(),
        };
        let mut retirement_identity = identity.clone();
        retirement_identity.conversation = run.attachment.as_ref().unwrap().conversation.clone();
        run.retirement = terminal_run(retirement_identity,
            RetirementState::Retired { at: now(), terminal }).retirement;
        let (terminal_index, protected, claim_bytes) = {
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs[0] = run;
            let attachment = state.runs[0].attachment.as_ref().unwrap();
            let holder = AttachmentReference { run_id: state.runs[0].run_id.clone(),
                attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch };
            claims::reserve_claim(&mut state, &checkout, &holder, &id()).unwrap();
            let run = state.runs[0].clone();
            let terminal_index = insert_resource(&mut state.owned_resources, &run,
                OwnedResourceIdentity::WorkerTerminal { identity }).unwrap();
            let git = OwnedGitIdentity {
                creation_operation_id: id(), machine_id: checkout.machine_id.clone(),
                repository_key: checkout.key.clone(), repository_root: checkout.path.clone(),
                repository_device: checkout.directory_identity.device.clone(),
                repository_inode: checkout.directory_identity.inode.clone(),
                common_dir: checkout.path.clone(), common_device: checkout.directory_identity.device.clone(),
                common_inode: checkout.directory_identity.inode.clone(), checkout_path: checkout.path.clone(),
                checkout_device: checkout.directory_identity.device.clone(),
                checkout_inode: checkout.directory_identity.inode.clone(), checkout_key: checkout.key.clone(),
                branch_ref: Some("refs/heads/owned".into()), creation_tip: "a".repeat(40),
            };
            let mut protected = Vec::new();
            for resource in [
                OwnedResourceIdentity::Worktree { identity: git.clone() },
                OwnedResourceIdentity::Branch { identity: git },
                OwnedResourceIdentity::Scratch { identity: OwnedFilesystemIdentity {
                    creation_operation_id: id(), machine_id: checkout.machine_id.clone(),
                    root: checkout.path.clone(), device: checkout.directory_identity.device.clone(),
                    inode: checkout.directory_identity.inode.clone(), manifest_path: checkout.path.clone(),
                    manifest_device: checkout.directory_identity.device.clone(),
                    manifest_inode: checkout.directory_identity.inode.clone(),
                } },
            ] {
                protected.push(insert_resource(&mut state.owned_resources, &run, resource).unwrap());
            }
            let claim_bytes = serde_json::to_vec(&state.checkout_claims).unwrap();
            sync_retirement_receipts(&mut state, 0);
            locked.save(&mut state).unwrap();
            (terminal_index, protected, claim_bytes)
        };
        let before = fixture.service.store.lock().unwrap().read().unwrap();
        let run = &before.runs[0];
        let attachment = run.attachment.as_ref().unwrap();
        assert!(!OrchestrationService::writer_quiescent(&before, run, &attachment.attachment_id, attachment.epoch));
        assert_ne!(before.checkout_claims[0].state, claims::ClaimState::Released);
        assert_eq!(serde_json::to_vec(&before.checkout_claims).unwrap(), claim_bytes);
        let terminal_resource = &before.owned_resources[terminal_index];
        assert_eq!(terminal_resource.cleanup.effects.len(), 1);
        assert_eq!(terminal_resource.cleanup.effects[0].state,
            if terminal == TerminalOutcome::ClosedByCockpit { CleanupEffectState::Applied } else { CleanupEffectState::Absent });
        assert_eq!(terminal_evaluation(terminal_resource, run), CleanupEvaluation::Absent);
        fixture.service.save_evaluation(terminal_resource, terminal_evaluation(terminal_resource, run)).unwrap();
        let after_terminal = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(after_terminal.owned_resources[terminal_index].cleanup.state, CleanupState::Removed);
        assert_eq!(serde_json::to_vec(&after_terminal.checkout_claims).unwrap(), claim_bytes);
        for index in protected {
            let resource = &after_terminal.owned_resources[index];
            assert!(!resource.consumers.is_empty());
            assert_eq!(OrchestrationService::consumers_unresolved(&after_terminal, resource), resource.consumers);
            if resource.kind != OwnedResourceKind::Scratch {
                assert!(OrchestrationService::known_writers_active(&after_terminal, resource));
            }
            assert!(fixture.service.begin_cleanup_effect(resource, run).unwrap().is_none(),
                "{:?}: retired pane must not admit a resource effect without consumer drain", resource.kind);
        }
        let after_attempts = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(serde_json::to_value(after_attempts).unwrap(), serde_json::to_value(after_terminal).unwrap(),
            "refused resource cleanup must not change claims, resources or journal state");
    }
}
