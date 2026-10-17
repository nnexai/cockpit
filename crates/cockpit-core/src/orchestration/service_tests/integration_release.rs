use super::*;
use crate::repositories::RepositoryCatalog;
use crate::space_git::tests::git;

// Exercise the persisted service boundary rather than manufacture OutputFacts.
// The supervisor stays in its Directory project; only the catalog knows main.
struct ReleasedIntegration {
    fixture: Fixture,
    root: String,
    supervisor: Actor,
    worker: String,
    task: Task,
    source: SourceOutput,
    target: PathBuf,
    source_path: PathBuf,
    catalog: RepositoryCatalog,
    target_commit: String,
}

impl ReleasedIntegration {
    async fn new() -> Self {
        let fixture = Fixture::new();
        let root = fixture.root();
        let supervisor = fixture.root_actor(&root);
        assert!(matches!(fixture.run(&root).project.unwrap().source, ProjectSource::Directory { .. }));
        let task = fixture.create_task(&root, "Integrate released code output");
        let worker = fixture.start(&root, &task);
        let target = fixture.root.join("repository");
        let source_path = fixture.root.join("checkouts").join(&worker);
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        git(&target, &["init", "-b", "main"]);
        git(&target, &["config", "user.name", "Integration service test"]);
        git(&target, &["config", "user.email", "integration-service@example.test"]);
        std::fs::write(target.join("contract.txt"), "baseline\n").unwrap();
        git(&target, &["add", "contract.txt"]);
        git(&target, &["commit", "-m", "baseline"]);
        let baseline = git(&target, &["rev-parse", "HEAD"]).trim().to_owned();
        git(&target, &["worktree", "add", "-b", "worker", source_path.to_str().unwrap(), &baseline]);
        std::fs::write(source_path.join("contract.txt"), "accepted implementation\n").unwrap();
        git(&source_path, &["add", "contract.txt"]);
        git(&source_path, &["commit", "-m", "accepted worker output"]);
        let head = git(&source_path, &["rev-parse", "HEAD"]).trim().to_owned();
        let (project, checkout) = project_identity::resolve_directory(&source_path, None, None).await.unwrap();
        fixture.setup(&worker);
        fixture.seed_state(|state| {
            let index = run_index(state, &worker).unwrap();
            state.runs[index].project = Some(project.clone());
            state.runs[index].setup.as_mut().unwrap().base_commit = Some(baseline.clone());
            let attachment = state.runs[index].attachment.as_ref().unwrap();
            let holder = AttachmentReference {
                run_id: worker.clone(), attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch,
            };
            claims::reserve_claim(state, &checkout, &holder, &id()).unwrap();
        });
        let worker_actor = fixture.launch_and_bind(&worker);
        fixture.initialized(&worker_actor);
        let message_id = id();
        let source = SourceOutput::Code {
            repository_key: project.key,
            checkout_key: checkout.key,
            base_commit: baseline,
            head_commit: head.clone(),
            commits: vec![head],
            result_message_id: message_id.clone(),
            task_revision: task.task_revision.clone(),
            focused_checks: vec![passed_check("source contract scenario")],
        };
        let mut action = report(ReportKind::Result, Some(ReportOutcome::Succeeded), None);
        if let OrchestrationAction::Report { message_id: result_id, source_output, .. } = &mut action {
            *result_id = message_id;
            *source_output = Some(source.clone());
        }
        let catalog = catalog_for(&fixture, &[&target]);
        let request = OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None, action };
        let output_facts = fixture.service.resolve_output_facts(&worker_actor, &request, &catalog).await.unwrap();
        assert!(output_facts.is_some());
        fixture.service.mutate_resolved(&worker_actor, request, &ResolvedMutationInput {
            output_facts, ..ResolvedMutationInput::default()
        }).unwrap();
        fixture.operator(OrchestrationAction::Accept {
            run_id: worker.clone(), expected_task_revision: task.task_revision.clone(),
        });
        git(&target, &["merge", "--ff-only", "worker"]);
        let target_commit = git(&target, &["rev-parse", "HEAD"]).trim().to_owned();
        fixture.seed_state(|state| {
            let run = state.runs[run_index(state, &worker).unwrap()].clone();
            let attachment = run.attachment.as_ref().unwrap();
            let claim = state.checkout_claims.iter().find(|claim|
                Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()).unwrap().clone();
            let holder = claim.holder.clone().unwrap();
            let revision = claims::seal_for_release(state, &claim.claim_id, &holder, claim.revision, &id()).unwrap();
            claims::release_with_proof(state, &claim.claim_id, &claims::WriterExitProof {
                incarnation: holder, claim_revision: revision, checkout: Some(claim.checkout),
                machine_id: attachment.machine_id.clone().unwrap(),
                previous_kernel_boot_id: attachment.process.as_ref().unwrap().kernel_boot_id.clone().unwrap(),
                observed_kernel_boot_id: "kernel-after-release".into(), all_consumers_absent: true,
                basis: claims::ExitBasis::Reboot,
            }).unwrap();
        });
        git(&target, &["worktree", "remove", source_path.to_str().unwrap()]);
        std::fs::write(target.join("draft.txt"), "unrelated target draft\n").unwrap();
        std::fs::write(target.join("contract.txt"), "unrelated target edit\n").unwrap();
        let run = fixture.run(&worker);
        assert_eq!(run.stage, RunStage::Closed);
        assert_eq!(run.close_reason, Some(CloseReason::Accepted));
        assert_eq!(run.integration, Integration::Pending);
        let claim = fixture.state().checkout_claims.into_iter().find(|claim|
            Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()).unwrap();
        assert_eq!(claim.state, claims::ClaimState::Released);
        assert!(claim.holder.is_none());
        assert!(!source_path.exists());
        Self { fixture, root, supervisor, worker, task, source, target, source_path, catalog, target_commit }
    }

    fn record(&self) -> IntegrationRecord {
        let operation_id = id();
        let SourceOutput::Code { repository_key, result_message_id, task_revision, .. } = &self.source else { unreachable!() };
        IntegrationRecord {
            operation_id: operation_id.clone(), run_id: self.worker.clone(),
            expected_result_message_id: result_message_id.clone(), expected_task_revision: task_revision.clone(),
            integration: Integration::Integrated {
                operation_id, accepted_result_message_id: result_message_id.clone(),
                accepted_task_revision: task_revision.clone(), source: self.source.clone(),
                target_repository_key: repository_key.clone(), target_ref: "refs/heads/main".into(),
                target_commit: self.target_commit.clone(), method: IntegrationMethod::Ancestor,
                mappings: Vec::new(), checks: vec![passed_check("target contract scenario")],
                integrator_run_id: self.root.clone(), verified_at: String::new(),
            },
        }
    }

    fn request(&self, actor: &Actor, record: IntegrationRecord) -> OrchestrationMutationRequest {
        OrchestrationMutationRequest {
            session_id: SESSION.into(), expected_revision: matches!(actor, Actor::Operator(_)).then(|| self.fixture.state().revision),
            action: OrchestrationAction::IntegrationRecord { record },
        }
    }

    async fn resolve(&self, actor: &Actor, request: &OrchestrationMutationRequest) -> ResolvedMutationInput {
        let before = self.fixture.state_bytes();
        let output_facts = self.fixture.service.resolve_output_facts(actor, request, &self.catalog).await.unwrap();
        assert!(output_facts.is_some());
        assert_eq!(self.fixture.state_bytes(), before, "resolution is read-only");
        ResolvedMutationInput { output_facts, ..ResolvedMutationInput::default() }
    }

    fn target_snapshot(&self) -> (String, String, String, Vec<u8>, Vec<u8>) {
        (
            git(&self.target, &["rev-parse", "HEAD"]),
            git(&self.target, &["show-ref"]),
            git(&self.target, &["diff", "--binary"]),
            std::fs::read(self.target.join("contract.txt")).unwrap(),
            std::fs::read(self.target.join("draft.txt")).unwrap(),
        )
    }
}

fn catalog_for(fixture: &Fixture, paths: &[&std::path::Path]) -> RepositoryCatalog {
    let mut configuration = fixture.configuration.clone();
    configuration.repository_roots = paths.iter().map(|path| path.to_str().unwrap().to_owned()).collect();
    configuration.limits.catalog_depth = 0;
    configuration.limits.catalog_entries = 32;
    configuration.limits.git_timeout_ms = 10_000;
    configuration.limits.git_output_bytes = 65_536;
    configuration.limits.operation_timeout_ms = 30_000;
    RepositoryCatalog::new(configuration)
}

fn passed_check(scenario: &str) -> CheckOutcome {
    CheckOutcome {
        contract: "accepted contract".into(), command_or_scenario: scenario.into(),
        outcome: CheckStatus::Passed, evidence: Some("fixture attestation at the exact recorded commit".into()), limitation: None,
    }
}

#[tokio::test]
async fn root_and_operator_record_released_removed_source_using_only_catalog_main() {
    for operator in [false, true] {
        let case = ReleasedIntegration::new().await;
        let actor = if operator { Actor::Operator(OperatorOrigin::Browser) } else { case.supervisor.clone() };
        let before_target = case.target_snapshot();
        assert!(case.fixture.state().runs.iter().all(|run|
            run.checkout.as_ref().is_none_or(|checkout| checkout.path != case.target.to_str().unwrap())),
            "main must be discovered from the catalog, never another run");
        let reopened = OrchestrationService::open(&case.fixture.configuration).unwrap();
        let request = case.request(&actor, case.record());
        let output_facts = reopened.resolve_output_facts(&actor, &request, &case.catalog).await.unwrap();
        let response = reopened.mutate_resolved(&actor, request.clone(), &ResolvedMutationInput {
            output_facts, ..ResolvedMutationInput::default()
        }).unwrap();
        let OrchestrationActionResult::IntegrationRecorded { receipt } = response.result else { panic!("expected integration receipt") };
        assert!(!receipt.duplicate);
        let Integration::Integrated { source, target_commit, verified_at, .. } = &receipt.integration else { panic!("expected verified integration") };
        assert_eq!(source, &case.source);
        assert_eq!(target_commit, &case.target_commit);
        assert!(!verified_at.is_empty());
        assert_eq!(case.fixture.run(&case.worker).integration, receipt.integration);
        assert!(case.fixture.task(&case.root, &case.task.task_id).checked);
        assert_eq!(case.target_snapshot(), before_target);
        git(&case.target, &["commit", "--allow-empty", "-m", "later unrelated target commit"]);
        let replay_target = case.target_snapshot();
        let reopened = OrchestrationService::open(&case.fixture.configuration).unwrap();
        let before = case.fixture.state_bytes();
        let mut replay = request;
        replay.expected_revision = matches!(&actor, Actor::Operator(_)).then(|| case.fixture.state().revision);
        // The public host path must also replay without reobserving a moved ref.
        let output_facts = reopened.resolve_output_facts(&actor, &replay, &case.catalog).await.unwrap();
        assert!(output_facts.is_none());
        let response = reopened.mutate_resolved(&actor, replay, &ResolvedMutationInput {
            output_facts, ..ResolvedMutationInput::default()
        }).unwrap();
        let OrchestrationActionResult::IntegrationRecorded { receipt: duplicate } = response.result else { panic!("expected replay receipt") };
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.integration, receipt.integration);
        assert_eq!(case.fixture.state_bytes(), before);
        assert_eq!(case.target_snapshot(), replay_target);
        assert!(!case.source_path.exists());
    }
}

#[tokio::test]
async fn released_integration_denies_cross_root_and_worker_callers_without_writes() {
    let case = ReleasedIntegration::new().await;
    let other_root = case.fixture.root();
    let cross_root = case.fixture.root_actor(&other_root);
    let (_, _, sibling) = case.fixture.working(&case.root);
    for actor in [cross_root, sibling] {
        let request = case.request(&actor, case.record());
        let before = case.fixture.state_bytes();
        let error = case.fixture.service.resolve_output_facts(&actor, &request, &case.catalog).await.unwrap_err();
        assert_eq!(error.code, "actor_forbidden");
        assert_code(case.fixture.service.mutate_resolved(&actor, request, &ResolvedMutationInput::default()), "actor_forbidden");
        assert_eq!(case.fixture.state_bytes(), before);
        assert_eq!(case.fixture.run(&case.worker).integration, Integration::Pending);
    }
}

#[tokio::test]
async fn released_claim_revision_changed_after_resolve_rejects_the_observation() {
    let case = ReleasedIntegration::new().await;
    let request = case.request(&case.supervisor, case.record());
    let resolved = case.resolve(&case.supervisor, &request).await;
    case.fixture.seed_state(|state| {
        let run = &state.runs[run_index(state, &case.worker).unwrap()];
        let claim = state.checkout_claims.iter_mut().find(|claim|
            Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()).unwrap();
        claim.revision += 1;
    });
    let before = case.fixture.state_bytes();
    assert_code(case.fixture.service.mutate_resolved(&case.supervisor, request, &resolved), "source_output_conflict");
    assert_eq!(case.fixture.state_bytes(), before);
    assert_eq!(case.fixture.run(&case.worker).integration, Integration::Pending);
}

#[tokio::test]
async fn target_ref_drift_before_or_after_resolution_never_commits_integration() {
    for after_resolve in [false, true] {
        let case = ReleasedIntegration::new().await;
        let request = case.request(&case.supervisor, case.record());
        let resolved = if after_resolve { Some(case.resolve(&case.supervisor, &request).await) } else { None };
        git(&case.target, &["-c", "core.hooksPath=/dev/null", "commit", "--allow-empty", "-m", "advance target after attestation"]);
        let target = case.target_snapshot();
        let before = case.fixture.state_bytes();
        if let Some(resolved) = resolved {
            assert!(case.fixture.service.mutate_resolved(&case.supervisor, request, &resolved).is_err());
        } else {
            assert!(case.fixture.service.resolve_output_facts(&case.supervisor, &request, &case.catalog).await.is_err());
        }
        assert_eq!(case.fixture.state_bytes(), before);
        assert_eq!(case.fixture.run(&case.worker).integration, Integration::Pending);
        assert_eq!(case.target_snapshot(), target);
    }
}

#[tokio::test]
async fn source_result_revision_and_current_generation_remain_cas_bound_after_resolution() {
    for change in 0..7 {
        let case = ReleasedIntegration::new().await;
        let request = case.request(&case.supervisor, case.record());
        let resolved = case.resolve(&case.supervisor, &request).await;
        if change == 6 {
            let locked = case.fixture.service.store.lock().unwrap();
            let document = locked.tasks(&case.root).unwrap();
            let accepted = document.task(&case.task.task_id).unwrap();
            document.update(&case.task.task_id, &accepted.task_revision,
                Some("Canonical scope changed after integration resolution"), None).unwrap();
        } else {
            case.fixture.seed_state(|state| {
                let index = run_index(state, &case.worker).unwrap();
                match change {
                    0 => {
                        if let Some(SourceOutput::Code { commits, .. }) = &mut state.runs[index].source_output { commits.clear(); }
                        state.runs[index].result.as_mut().unwrap().source_output = state.runs[index].source_output.clone();
                    }
                    1 => state.runs[index].result.as_mut().unwrap().message_id = id(),
                    2 => {
                        if let Some(SourceOutput::Code { task_revision, .. }) = &mut state.runs[index].source_output { *task_revision = "different revision".into(); }
                        state.runs[index].result.as_mut().unwrap().source_output = state.runs[index].source_output.clone();
                    }
                    3 => {
                        let ownership = state.task_ownership.iter_mut().find(|ownership|
                            ownership.root_id == case.root && ownership.task_id == case.task.task_id).unwrap();
                        ownership.generation += 1;
                    }
                    4 => {
                        let ownership = state.task_ownership.iter_mut().find(|ownership|
                            ownership.root_id == case.root && ownership.task_id == case.task.task_id).unwrap();
                        ownership.generation += 1;
                        state.runs[index].assignment_generation = Some(ownership.generation);
                    }
                    5 => state.runs[index].result.as_mut().unwrap().source_output = None,
                    _ => unreachable!(),
                }
            });
        }
        let before = case.fixture.state_bytes();
        assert!(case.fixture.service.mutate_resolved(&case.supervisor, request, &resolved).is_err(), "mutation {change} reused stale proof");
        assert_eq!(case.fixture.state_bytes(), before);
        assert_eq!(case.fixture.run(&case.worker).integration, Integration::Pending);
    }
}

#[tokio::test]
async fn missing_failed_or_unrelated_target_checks_persist_pending_not_source_success() {
    for change in 0..5 {
        let case = ReleasedIntegration::new().await;
        let mut record = case.record();
        if let Integration::Integrated { checks, .. } = &mut record.integration {
            match change {
                0 => checks.clear(),
                1 => checks[0].outcome = CheckStatus::Failed,
                2 => checks[0].outcome = CheckStatus::NotRun,
                3 => checks[0].contract = "unrelated target contract".into(),
                4 => checks[0].limitation = Some("target scenario not actually completed".into()),
                _ => unreachable!(),
            }
        }
        let request = case.request(&case.supervisor, record);
        let resolved = case.resolve(&case.supervisor, &request).await;
        let target = case.target_snapshot();
        let response = case.fixture.service.mutate_resolved(&case.supervisor, request, &resolved).unwrap();
        let OrchestrationActionResult::IntegrationRecorded { receipt } = response.result else { panic!("expected Pending receipt") };
        assert_eq!(receipt.integration, Integration::Pending);
        let reopened = OrchestrationService::open(&case.fixture.configuration).unwrap();
        let state = reopened.store.lock().unwrap().read().unwrap();
        assert_eq!(state.runs[run_index(&state, &case.worker).unwrap()].integration, Integration::Pending);
        assert_eq!(case.target_snapshot(), target);
    }
}

#[tokio::test]
async fn catalog_cannot_substitute_wrong_repository_or_sibling_worktree_for_main() {
    let case = ReleasedIntegration::new().await;
    let sibling = case.fixture.root.join("sibling-checkout");
    git(&case.target, &["worktree", "add", "-b", "sibling", sibling.to_str().unwrap(), "HEAD"]);
    let wrong = case.fixture.root.join("wrong-repository");
    std::fs::create_dir_all(&wrong).unwrap();
    git(&wrong, &["init", "-b", "main"]);
    git(&wrong, &["-c", "user.name=Other", "-c", "user.email=other@example.test", "commit", "--allow-empty", "-m", "unrelated repository"]);
    let request = case.request(&case.supervisor, case.record());
    for path in [&wrong, &sibling] {
        let catalog = catalog_for(&case.fixture, &[path]);
        let before = case.fixture.state_bytes();
        assert!(case.fixture.service.resolve_output_facts(&case.supervisor, &request, &catalog).await.is_err(),
            "only the requested repository's catalog main checkout may be selected");
        assert_eq!(case.fixture.state_bytes(), before);
    }
    // Even a live same-root sibling checkout is not the integrator's main target.
    let (_, sibling_run, _) = case.fixture.working(&case.root);
    let (project, checkout) = project_identity::resolve_directory(&sibling, None, None).await.unwrap();
    case.fixture.seed_run(&sibling_run, |run| {
        run.project = Some(project);
        run.checkout = Some(checkout);
    });
    let empty_catalog = catalog_for(&case.fixture, &[]);
    let before = case.fixture.state_bytes();
    assert!(case.fixture.service.resolve_output_facts(&case.supervisor, &request, &empty_catalog).await.is_err());
    assert_eq!(case.fixture.state_bytes(), before);
    let resolved = case.resolve(&case.supervisor, &request).await;
    assert!(case.fixture.service.mutate_resolved(&case.supervisor, request, &resolved).is_ok());
}

#[tokio::test]
async fn automatic_reconciliation_of_released_source_requires_passed_exact_target_receipt() {
    for target_checks in 0..3 {
        let case = ReleasedIntegration::new().await;
        let (_, checked_run, _) = case.fixture.working(&case.root);
        let (project, checkout) = project_identity::resolve_directory(&case.target, None, None).await.unwrap();
        // Automatic owner reconciliation has no catalog input: give it a known
        // root/main identity, not a sibling worker checkout as a substitute.
        case.fixture.seed_run(&case.root, |run| {
            run.project = Some(project);
            run.checkout = Some(checkout);
        });
        if target_checks != 0 {
            let mut receipt = case.record().integration;
            if let Integration::Integrated { checks, verified_at, .. } = &mut receipt {
                *verified_at = now();
                if target_checks == 1 { checks[0].outcome = CheckStatus::Failed; }
            }
            case.fixture.seed_run(&checked_run, |run| run.integration = receipt);
        }
        let before = case.fixture.state_bytes();
        let target = case.target_snapshot();
        let reopened = OrchestrationService::open(&case.fixture.configuration).unwrap();
        reopened.reconcile_accepted_integrations().await.unwrap();
        let integration = case.fixture.run(&case.worker).integration;
        if target_checks == 2 {
            assert!(matches!(integration, Integration::Integrated {
                method: IntegrationMethod::Ancestor, source, target_commit, ..
            } if source == case.source && target_commit == case.target_commit));
        } else {
            assert_eq!(integration, Integration::Pending);
            assert_eq!(case.fixture.state_bytes(), before);
        }
        assert_eq!(case.target_snapshot(), target);
        assert!(!case.source_path.exists());
    }
}
