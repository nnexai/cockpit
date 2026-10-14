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
        let service = OrchestrationService { store, revision, notifications: std::sync::Mutex::new(None), launch_config: Default::default() };
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
    let intent = fixture.service.begin_cleanup_effect(&fixture.resource, &fixture.run, None).unwrap().unwrap();
    let reopened = OrchestrationStore::open(&fixture.root).unwrap();
    let saved = reopened.lock().unwrap().read().unwrap();
    assert_eq!(saved.owned_resources[0].cleanup.state, CleanupState::Running);
    assert_eq!(saved.owned_resources[0].cleanup.effects[0].state, CleanupEffectState::Intent);
    assert!(fixture.service.begin_cleanup_effect(&intent, &fixture.run, None).unwrap().is_none());
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
fn local_checkout_writers_protect_owned_git_by_path_or_directory_fingerprint() {
    let run = worker();
    let mut state = OrchestrationState::default();
    let index = insert_resource(&mut state.owned_resources, &run, OwnedResourceIdentity::Worktree {
        identity: OwnedGitIdentity {
            creation_operation_id: id(), machine_id: "physical-receipt-machine".into(),
            repository_key: "/retained/.git".into(), repository_root: "/retained".into(),
            repository_device: "1".into(), repository_inode: "2".into(),
            common_dir: "/retained/.git".into(), common_device: "1".into(), common_inode: "3".into(),
            checkout_path: "/owned/checkout".into(), checkout_device: "1".into(), checkout_inode: "4".into(),
            checkout_key: "owned-checkout-key".into(), branch_ref: Some("refs/heads/owned".into()),
            creation_tip: "a".repeat(40),
        },
    }).unwrap();
    let resource = state.owned_resources[index].clone();
    for (path, device, inode, protected) in [
        ("/owned/checkout/sub", "9", "10", true),
        ("/another/local/path", "1", "4", true),
        ("/owned/checkout-neighbor", "9", "10", false),
    ] {
        state.checkout_claims = vec![claims::CheckoutClaim {
            claim_id: id(), checkout: CheckoutIdentity {
                key: "distinct-local-key".into(), path: path.into(),
                directory_identity: DirectoryIdentity { device: device.into(), inode: inode.into() }, git: None,
            },
            revision: 1, state: claims::ClaimState::Held, holder: None, transition_operation_id: None,
        }];
        assert_eq!(OrchestrationService::known_writers_active(&state, &resource), protected);
        state.checkout_claims[0].state = claims::ClaimState::Released;
        assert!(!OrchestrationService::known_writers_active(&state, &resource));
    }
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
        assert!(fixture.service.begin_cleanup_effect(&expected, &fixture.run, None).unwrap().is_none());
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
        herdr_peer: None,
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
        result_message_id: Some("accepted-result".into()), task_revision: Some("accepted-revision".into()), replacement: None,
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
                peer_process: identity.herdr_peer.clone(),
                native_session_id: None, endpoint_identity: identity.endpoint_identity.clone(),
                session_id: identity.session_id.clone(), workspace_id: identity.workspace_id.clone(),
                tab_id: identity.tab_id.clone(), pane_id: identity.pane_id.clone(), launch_tag: identity.launch_tag.clone(),
            }),
            conversation: Some(ConversationIdentity { harness: "omp".into(), session_id: "accepted-chat".into() }),
            process: identity.process.clone(), shell: identity.shell.clone(),
            machine_id: Some("test-machine".into()), launch_attempt: identity.launch_attempt, updated_at: now(),
            process_scope: None,
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
                creation_operation_id: id(), machine_id: "test-machine".into(),
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
                    creation_operation_id: id(), machine_id: "test-machine".into(),
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
            assert!(fixture.service.begin_cleanup_effect(resource, run, None).unwrap().is_none(),
                "{:?}: retired pane must not admit a resource effect without consumer drain", resource.kind);
        }
        let after_attempts = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(serde_json::to_value(after_attempts).unwrap(), serde_json::to_value(after_terminal).unwrap(),
            "refused resource cleanup must not change claims, resources or journal state");
    }
}

#[test]
fn event_queue_keeps_superseded_resources_separate_from_working_successor() {
    let fixture = Fixture::new();
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.runs[0].close_reason = Some(CloseReason::Superseded);
        let mut successor = state.runs[0].clone();
        successor.run_id = id();
        successor.stage = RunStage::Working;
        successor.close_reason = None;
        successor.attempt += 1;
        let next = insert_resource(&mut state.owned_resources, &successor, space()).unwrap();
        state.owned_resources[next].cleanup.operation_id = Some(id());
        state.runs.push(successor);
        locked.save(&mut state).unwrap();
    }
    let queue = fixture.service.cleanup_queue().unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].run_id, fixture.run.run_id);
    assert_eq!(queue[0].resource_id, fixture.resource.resource_id);
}

#[test]
fn cleanup_hold_is_actionable_once_after_ack_and_owner_reopen() {
    let fixture = Fixture::new();
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let mut root = worker();
        root.run_id = fixture.run.root_id.clone();
        root.root_id = root.run_id.clone();
        root.kind = RunKind::Supervisor;
        root.parent_run_id = None;
        root.stage = RunStage::Active;
        state.runs.push(root);
        locked.save(&mut state).unwrap();
    }
    fixture.service.save_evaluation(&fixture.resource, CleanupEvaluation::Waiting(
        reason(CleanupReasonClass::Reconcilable, "integration_pending", "Missing passed checks at the exact target commit"))).unwrap();
    fixture.service.report_cleanup_holds().unwrap();
    let initial = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(initial.messages.len(), 1);
    assert_eq!(initial.messages[0].kind, MessageKind::Observation);
    assert!(matches!(initial.messages[0].from, ActorRef::Dispatcher));
    assert!(initial.messages[0].text.contains("IntegrationRecord"));
    assert!(initial.messages[0].text.contains(&fixture.run.run_id));
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.messages[0].stage = DeliveryStage::Acked;
        state.messages[0].acked_at = Some(now());
        locked.save(&mut state).unwrap();
    }
    let (revision, _) = watch::channel(0);
    let reopened = OrchestrationService { store: OrchestrationStore::open(&fixture.root).unwrap(),
        revision, notifications: std::sync::Mutex::new(None), launch_config: Default::default() };
    let before = std::fs::read(reopened.base().join("state.json")).unwrap();
    reopened.report_cleanup_holds().unwrap();
    assert_eq!(std::fs::read(reopened.base().join("state.json")).unwrap(), before);
    let saved = reopened.store.lock().unwrap().read().unwrap();
    let resource = saved.owned_resources[0].clone();
    reopened.save_evaluation(&resource, CleanupEvaluation::Unknown(
        reason(CleanupReasonClass::Reconcilable, "effect_outcome_unknown", "Uncertain deletion preserved"))).unwrap();
    reopened.report_cleanup_holds().unwrap();
    let saved = reopened.store.lock().unwrap().read().unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert!(saved.messages[1].text.contains("do not resubmit"));
}

#[test]
fn transient_observation_retention_is_waiting_with_concrete_reconnection_action() {
    for (retained, code) in [
        (RetainReason::ObservationUnavailable, "terminal_observation_unavailable"),
        (RetainReason::ForegroundProcess, "terminal_retirement_pending"),
    ] {
        let identity = terminal_identity();
        let run = terminal_run(identity.clone(), RetirementState::Retained {
            at: now(), reason: retained, stopped: false,
        });
        let mut state = OrchestrationState::default();
        let index = insert_resource(&mut state.owned_resources, &run,
            OwnedResourceIdentity::WorkerTerminal { identity }).unwrap();
        assert!(matches!(terminal_evaluation(&state.owned_resources[index], &run),
            CleanupEvaluation::Waiting(why) if why.code == code));
    }
}

#[test]
fn primary_checkout_refusal_settles_receipt_and_reports_authority_hold_without_touching_spaces() {
    let fixture = Fixture::new();
    let identity = terminal_identity();
    let mut run = terminal_run(identity.clone(), RetirementState::CloseIntent { at: now() });
    run.run_id = fixture.run.run_id.clone();
    run.root_id = fixture.run.root_id.clone();
    let terminal_index = {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.runs[0] = run.clone();
        let mut root = worker();
        root.run_id = run.root_id.clone();
        root.root_id = root.run_id.clone();
        root.kind = RunKind::Supervisor;
        root.parent_run_id = None;
        root.stage = RunStage::Active;
        state.runs.push(root);
        let index = insert_resource(&mut state.owned_resources, &run,
            OwnedResourceIdentity::WorkerTerminal { identity: identity.clone() }).unwrap();
        sync_retirement_receipts(&mut state, 0);
        assert_eq!(state.owned_resources[index].cleanup.effects[0].state, CleanupEffectState::Intent);
        locked.save(&mut state).unwrap();
        index
    };
    let protected_space = fixture.service.store.lock().unwrap().read().unwrap().owned_resources[0].clone();
    run.retirement.as_mut().unwrap().state = retirement::classify_close(&identity,
        Some(Err(error("foreign_space_group_close_risk", "opaque adapter diagnostic"))), None);
    let resource = {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.runs[0] = run.clone();
        sync_retirement_receipts(&mut state, 0);
        let resource = state.owned_resources[terminal_index].clone();
        assert_eq!(resource.cleanup.effects.len(), 1);
        assert_eq!(resource.cleanup.effects[0].state, CleanupEffectState::Refused);
        assert_eq!(state.owned_resources[0], protected_space);
        locked.save(&mut state).unwrap();
        resource
    };
    let evaluation = terminal_evaluation(&resource, &run);
    assert!(matches!(&evaluation, CleanupEvaluation::Kept(why)
        if why.class == CleanupReasonClass::Authority && why.code == "terminal_primary_checkout_space"));
    fixture.service.save_evaluation(&resource, evaluation).unwrap();
    fixture.service.report_cleanup_holds().unwrap();
    let saved = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(saved.owned_resources[0], protected_space);
    assert_eq!(saved.owned_resources[terminal_index].cleanup.state, CleanupState::Kept);
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].message_id,
        format!("cleanup:{}:terminal_primary_checkout_space", resource.resource_id));
    fixture.service.report_cleanup_holds().unwrap();
    let unchanged = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(serde_json::to_value(unchanged).unwrap(), serde_json::to_value(saved).unwrap());
}

#[test]
fn explicit_supervisor_finish_replay_is_inert_and_preserves_unknown_effect() {
    let fixture = Fixture::new();
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let mut root = worker();
        root.run_id = fixture.run.root_id.clone();
        root.root_id = root.run_id.clone();
        root.kind = RunKind::Supervisor;
        root.parent_run_id = None;
        root.stage = RunStage::Active;
        state.runs.push(root);
        state.owned_resources[0].cleanup.state = CleanupState::Unknown;
        state.owned_resources[0].cleanup.effects.push(CleanupEffectReceipt {
            effect_id: id(), kind: OwnedResourceKind::Space, identity_digest: fixture.resource.resource_id.clone(),
            state: CleanupEffectState::Unknown, evidence: None, updated_at: now(),
        });
        locked.save(&mut state).unwrap();
    }
    let action = OrchestrationAction::CleanupFinish { request: CleanupFinish {
        operation_id: id(), root_id: fixture.run.root_id.clone(), run_id: Some(fixture.run.run_id.clone()),
    }};
    let mutate = || fixture.service.mutate(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationMutationRequest { session_id: fixture.run.session_id.clone(), expected_revision: None,
            action: action.clone() }).unwrap();
    let first = mutate();
    assert!(matches!(first.result, OrchestrationActionResult::CleanupFinished { cleanup }
        if !cleanup.duplicate && cleanup.unknown_count == 1));
    let before = std::fs::read(fixture.service.base().join("state.json")).unwrap();
    let replay = mutate();
    assert!(matches!(replay.result, OrchestrationActionResult::CleanupFinished { cleanup }
        if cleanup.duplicate && cleanup.unknown_count == 1));
    assert_eq!(std::fs::read(fixture.service.base().join("state.json")).unwrap(), before);
    let saved = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(saved.owned_resources[0].cleanup.state, CleanupState::Unknown);
    assert_eq!(saved.owned_resources[0].cleanup.effects[0].state, CleanupEffectState::Unknown);
}

#[test]
fn setup_capture_waits_for_its_exact_later_bound_space_receipt() {
    let mut run = worker();
    let mut source_space = space();
    let OwnedResourceIdentity::Space { identity } = &mut source_space else { unreachable!() };
    let operation = identity.creation_operation_id.clone();
    identity.workspace_id = "initial-source".into();
    run.setup = Some(SetupSummary { operation_id: Some(operation), generation: None, workspace_id: None,
        checkout_path: "/owned/checkout".into(), repository_id: None, branch: None, base: None,
        base_commit: None, ownership: None, effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None });
    let mut resources = Vec::new();
    insert_resource(&mut resources, &run, source_space).unwrap();
    assert!(setup_resources_captured(&resources, &run));
    run.setup.as_mut().unwrap().workspace_id = Some("bound-worker".into());
    assert!(!setup_resources_captured(&resources, &run), "early setup resources cannot suppress later Space capture");
    let mut bound = space();
    let OwnedResourceIdentity::Space { identity } = &mut bound else { unreachable!() };
    identity.workspace_id = "bound-worker".into();
    assert_ne!(Some(identity.creation_operation_id.as_str()), run.setup.as_ref().unwrap().operation_id.as_deref());
    insert_resource(&mut resources, &run, bound).unwrap();
    assert!(setup_resources_captured(&resources, &run), "exact moved-Space association completes capture despite its distinct operation ID");
    run.setup.as_mut().unwrap().ownership = Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree);
    let space_only = [resources.last().unwrap().clone()];
    assert!(!setup_resources_captured(&space_only, &run), "owned Git checkout capture still requires its original setup receipt");
    run.setup.as_mut().unwrap().ownership = Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory);
    assert!(setup_resources_captured(&space_only, &run), "borrowed directory preparation owns only its exact new moved Space, never the directory or Git");
    let old_receipts = serde_json::to_value(&resources).unwrap();
    let original_creator = run.setup.as_ref().unwrap().operation_id.clone();
    let resume_move = id();
    run.dispatch = Some(dispatch(DispatchStep::Launched));
    run.dispatch.as_mut().unwrap().move_operation_id = Some(resume_move.clone());
    assert!(!setup_resources_captured(&resources, &run),
        "a matching workspace from a different creation operation cannot mask a new epoch's move binding");
    run.setup.as_mut().unwrap().workspace_id = Some("resumed-worker".into());
    let mut resumed = space();
    let OwnedResourceIdentity::Space { identity } = &mut resumed else { unreachable!() };
    identity.workspace_id = "resumed-worker".into();
    identity.creation_operation_id = resume_move.clone();
    insert_resource(&mut resources, &run, resumed).unwrap();
    assert!(setup_resources_captured(&resources, &run));
    assert_eq!(run.setup.as_ref().unwrap().operation_id, original_creator,
        "new Space binding never replaces the original filesystem creator");
    assert_eq!(serde_json::to_value(&resources[..resources.len() - 1]).unwrap(), old_receipts,
        "resumed capture appends new ownership without rewriting any old resource receipt");
    assert_eq!(resources.last().unwrap().creation_operation_id.as_deref(), Some(resume_move.as_str()));
}

#[test]
fn borrowed_resume_capture_grants_only_exact_new_space_without_filesystem_creator() {
    let fixture = Fixture::new();
    let (_, checkout) = project_identity::resolve_owned_directory(&fixture.root).unwrap();
    let mut run = worker();
    run.checkout = Some(checkout.clone());
    run.setup = Some(SetupSummary { operation_id: None, generation: None, workspace_id: Some("new-borrowed-space".into()),
        checkout_path: checkout.path.clone(), repository_id: None, branch: None, base: None, base_commit: None,
        ownership: None, effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None });
    run.dispatch = Some(dispatch(DispatchStep::Launched));
    let move_operation = id();
    run.dispatch.as_mut().unwrap().move_operation_id = Some(move_operation.clone());
    let mut moved = space();
    let OwnedResourceIdentity::Space { identity } = &mut moved else { unreachable!() };
    identity.creation_operation_id = move_operation.clone();
    identity.workspace_id = "new-borrowed-space".into();
    identity.checkout_path = checkout.path.clone();
    let mut resources = Vec::new();
    assert!(!setup_resources_captured(&resources, &run));
    insert_resource(&mut resources, &run, moved).unwrap();
    assert!(setup_resources_captured(&resources, &run));
    assert!(run.setup.as_ref().unwrap().operation_id.is_none(), "borrowed filesystem never receives a fabricated creator");
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].kind, OwnedResourceKind::Space);
    assert_eq!(resources[0].creation_operation_id.as_deref(), Some(move_operation.as_str()));
    for changed in 0..3 {
        let mut wrong = run.clone();
        match changed {
            0 => wrong.checkout.as_mut().unwrap().path = "/different-borrowed-checkout".into(),
            1 => wrong.dispatch.as_mut().unwrap().move_operation_id = Some(id()),
            _ => wrong.setup.as_mut().unwrap().workspace_id = Some("old-space".into()),
        }
        assert!(!setup_resources_captured(&resources, &wrong));
    }
    project_identity::revalidate_checkout(&checkout).unwrap();
}

#[tokio::test]
async fn scheduled_inputs_keep_git_ref_wakes_after_predecessor_resources_are_removed() {
    use crate::space_git::tests::git;
    let fixture = Fixture::new();
    let repository = fixture.root.join("source");
    std::fs::create_dir(&repository).unwrap();
    git(&repository, &["init", "-b", "main"]);
    let project = project_identity::resolve_directory(&repository, None, None).await.unwrap().0;
    let ProjectSource::Git { common_dir } = &project.source else { unreachable!() };
    let expected = PathBuf::from(common_dir);
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.runs[0].project = Some(project);
        state.runs[0].integration = Integration::NotApplicable { reason: "test completed".into(), result_message_id: "result".into() };
        state.owned_resources[0].cleanup.state = CleanupState::Removed;
        state.task_ownership.push(TaskOwnership { root_id: fixture.run.root_id.clone(), task_id: id(),
            generation: 0, owner: None, schedule: Some(TaskSchedule { operation_id: id(), principal_key: "root".into(),
                target: None, brief: "waiting on actual integrated input".into(), cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe,
                label: None, scheduled_at: now() }) });
        locked.save(&mut state).unwrap();
    }
    assert_eq!(fixture.service.cleanup_ref_watch_paths().unwrap(), vec![expected]);
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.task_ownership[0].schedule = None;
        locked.save(&mut state).unwrap();
    }
    assert!(fixture.service.cleanup_ref_watch_paths().unwrap().is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn terminal_capture_defers_preexec_and_stale_cache_then_preserves_exact_postexec_creation() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    use super::super::herdr::{AgentTabRequest, OrchestrationHerdr, PaneProcessInfo, RuntimePane, RuntimeView};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CaptureAdapter { runtime: RuntimeView, info: PaneProcessInfo, snapshots: AtomicUsize }
    #[async_trait::async_trait]
    impl OrchestrationHerdr for CaptureAdapter {
        async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> {
            self.snapshots.fetch_add(1, Ordering::SeqCst); Ok(self.runtime.clone())
        }
        async fn pane_process_info(&self, _: &str, endpoint: &str, pane: &str) -> Result<PaneProcessInfo, InspectionError> {
            assert_eq!(endpoint, self.runtime.endpoint_identity); assert_eq!(pane, self.info.pane_id); Ok(self.info.clone())
        }
        async fn create_agent_tab(&self, _: &str, _: &AgentTabRequest) -> Result<RunLocation, InspectionError> { panic!("capture never launches") }
        async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { panic!("capture never closes") }
    }
    struct OwnedChild(std::process::Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }
    // Same native transition fixture as process_identity's exec regression:
    // the private latch shell execs without changing PID/start identity.
    let mut child = OwnedChild(Command::new("sh")
        .args(["-c", "printf 'ready\\n'; read line; exec sleep 60"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap());
    let mut ready = String::new();
    BufReader::new(child.0.stdout.take().unwrap()).read_line(&mut ready).unwrap();
    assert_eq!(ready, "ready\n");
    let pid = i32::try_from(child.0.id()).unwrap();
    let start = crate::process_identity::start_identity(pid).unwrap();
    let preexec = crate::process_identity::executable_identity(pid, start).unwrap();
    let peer_pid = std::process::id();
    let peer = NativeProcessIdentity { pid: peer_pid,
        start_ticks: crate::process_identity::start_identity(i32::try_from(peer_pid).unwrap()).unwrap(),
        kernel_boot_id: crate::process_identity::kernel_boot_id() };
    let mut run = worker();
    run.stage = RunStage::Starting;
    run.close_reason = None;
    run.task_id = Some(id());
    run.assignment_generation = Some(1);
    run.dispatch = Some(dispatch(DispatchStep::Launched));
    let dispatch = run.dispatch.as_mut().unwrap();
    dispatch.launch_attempt = 1;
    dispatch.launch_tag = Some("private-exec-transition".into());
    dispatch.endpoint_identity = Some("endpoint".into());
    run.attachment = Some(ExecutionAttachment {
        attachment_id: id(), epoch: 1, state: AttachmentState::Connected,
        launch_mode: Some(AttachmentLaunchMode::DirectProcess),
        location: Some(RunLocation { endpoint_identity: "endpoint".into(), session_id: run.session_id.clone(),
            workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: Some("terminal".into()),
            launch_tag: "private-exec-transition".into(), boot_id: None,
            peer_process: Some(peer.clone()), native_session_id: None }),
        conversation: None, process: Some(preexec.process.clone()), shell: Some(preexec.clone()),
        machine_id: crate::process_identity::machine_id(), process_scope: None, launch_attempt: 1, updated_at: now(),
    });
    let mut state = OrchestrationState::default();
    state.credentials.push(credentials::test_job_credential(&run, JobRole::Worker).0);
    state.task_ownership.push(TaskOwnership { root_id: run.root_id.clone(), task_id: run.task_id.clone().unwrap(),
        generation: 1, owner: Some(TaskOwner { run_id: run.run_id.clone(), conversation: None,
            claim_operation_id: state.credentials[0].credential_id.clone() }), schedule: None });
    assert!(retirement::identity_for(&run).is_some(), "the old ungated path could freeze the latch fingerprint");
    assert!(OrchestrationService::terminal_capture_identity(&state, &run).is_none());
    child.0.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let postexec = loop {
        if let Ok(fresh) = crate::process_identity::executable_identity(pid, start) {
            if fresh != preexec { break fresh; }
        }
        assert!(Instant::now() < deadline, "private child did not exec");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(postexec.process, preexec.process);
    run.stage = RunStage::Working;
    assert!(has_current_binding(&state, &run));
    assert!(OrchestrationService::terminal_capture_identity(&state, &run).is_none(),
        "active semantics do not excuse a stale preexec cached fingerprint");
    run.attachment.as_mut().unwrap().shell = Some(postexec.clone());
    let captured = OrchestrationService::terminal_capture_identity(&state, &run).unwrap();
    assert_eq!(captured.shell.as_ref(), Some(&postexec));
    let adapter = CaptureAdapter { runtime: RuntimeView { endpoint_identity: captured.endpoint_identity.clone(),
        boot_id: None, peer_process: Some(peer), focused_workspace_id: None, workspaces: Vec::new(),
        panes: vec![RuntimePane { workspace_id: captured.workspace_id.clone(), workspace_label: "space".into(),
            tab_id: captured.tab_id.clone(), tab_label: captured.launch_tag.clone(), pane_id: captured.pane_id.clone(),
            cwd: None, terminal_id: Some(captured.terminal_id.clone()), native_session_id: None, native_session_file: None,
            agent_name: None, agent_kind: None, launch_pending: false, agent_status: Some("working".into()), state_changed_at: None }] },
        info: PaneProcessInfo { pane_id: captured.pane_id.clone(), shell_pid: Some(postexec.process.pid),
            foreground_pgid: Some(postexec.process.pid), processes: vec![(postexec.process.pid, "real exec child".into())],
            shell_identity: Some(postexec.clone()) }, snapshots: AtomicUsize::new(0) };
    assert!(OrchestrationService::terminal_capture_observed(&adapter, &run, &captured).await);
    assert_eq!(adapter.snapshots.load(Ordering::SeqCst), 1, "one fresh frame for the pending capture");
    for changed in 0..5 {
        let mut bad = CaptureAdapter { runtime: adapter.runtime.clone(), info: adapter.info.clone(), snapshots: AtomicUsize::new(0) };
        match changed {
            0 => bad.runtime.endpoint_identity = "different endpoint".into(),
            1 => bad.runtime.peer_process = None,
            2 => bad.runtime.peer_process.as_mut().unwrap().start_ticks += 1,
            3 => bad.runtime.panes[0].terminal_id = Some("different terminal".into()),
            _ => bad.info.shell_identity = Some(preexec.clone()),
        }
        assert!(!OrchestrationService::terminal_capture_observed(&bad, &run, &captured).await);
    }
    insert_resource(&mut state.owned_resources, &run, OwnedResourceIdentity::WorkerTerminal { identity: captured.clone() }).unwrap();
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(crate::process_identity::executable_identity(pid, start).is_err());
    run.stage = RunStage::Closed;
    run.close_reason = Some(CloseReason::Accepted);
    run.result = Some(Report { message_id: "actual-result".into(), kind: ReportKind::Result,
        outcome: Some(ReportOutcome::Succeeded), summary: "actual completed test child".into(),
        plan: None, source_output: None, at: now() });
    run.retirement = retirement::retirement_for_accepted(&run, RetirementTrigger::Accept, "revision", &now());
    run.retirement.as_mut().unwrap().state = RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent };
    assert_eq!(OrchestrationService::terminal_capture_identity(&state, &run), Some(captured.clone()));
    assert!(!OrchestrationService::terminal_capture_observed(&adapter, &run, &captured).await,
        "closed attempts cannot adopt a still-present mutable pane");
    let mut absent = adapter;
    absent.runtime.panes.clear();
    assert!(OrchestrationService::terminal_capture_observed(&absent, &run, &captured).await);
    assert_eq!(terminal_evaluation(&state.owned_resources[0], &run), CleanupEvaluation::Absent);
    run.attachment.as_mut().unwrap().shell = Some(preexec);
    assert!(OrchestrationService::terminal_capture_identity(&state, &run).is_none(),
        "closed capture cannot backfill the immutable pin from a different mutable attachment");
    assert_eq!(state.owned_resources.len(), 1);
    let OwnedResourceIdentity::WorkerTerminal { identity } = &state.owned_resources[0].identity else { unreachable!() };
    assert_eq!(identity.shell.as_ref(), Some(&postexec));
}
