use super::*;
use super::merged::merged_proof;
use crate::project_adapter::{capture_owned_git_identity, remove_owned_branch, remove_owned_worktree};
use crate::space_git::tests::git;
use std::path::{Path, PathBuf};

const TIMEOUT_MS: u64 = 10_000;
const OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Default)]
struct NoEffectsAdapter;

#[async_trait::async_trait]
impl crate::HerdrAdapter for NoEffectsAdapter {
    async fn inspect(&self) -> Result<cockpit_protocol::v1::HerdrCompatibility, InspectionError> { panic!("unexpected Herdr call") }
    async fn inspect_session(&self, _: &str) -> Result<cockpit_protocol::v1::HerdrCompatibility, InspectionError> { panic!("unexpected Herdr call") }
    async fn sessions(&self) -> Result<cockpit_protocol::v1::SessionListResponse, InspectionError> { panic!("unexpected Herdr call") }
    async fn session_snapshot(&self, _: &str) -> Result<cockpit_protocol::v1::SessionSnapshotResponse, InspectionError> { panic!("unexpected Herdr call") }
    async fn focus(&self, _: &str, _: &cockpit_protocol::v1::FocusRequest) -> Result<cockpit_protocol::v1::FocusResponse, InspectionError> { panic!("unexpected Herdr call") }
    async fn mutate(&self, _: &str, _: &cockpit_protocol::v1::ResourceMutationRequest) -> Result<cockpit_protocol::v1::ResourceMutationResponse, InspectionError> { panic!("unexpected Herdr call") }
    async fn subscribe_session(&self, _: &str, _: &cockpit_protocol::v1::SessionSnapshotResponse) -> Result<crate::SessionSubscription, InspectionError> { panic!("unexpected Herdr call") }
    async fn open_terminal(&self, _: &cockpit_protocol::v1::TerminalOpenRequest) -> Result<crate::TerminalSession, InspectionError> { panic!("unexpected Herdr call") }
}

#[async_trait::async_trait]
impl crate::ProjectHerdrAdapter for NoEffectsAdapter {
    async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> { panic!("unexpected project call") }
    async fn project_inventory(&self, _: &str, _: &str) -> Result<crate::project_adapter::ProjectInventory, InspectionError> { panic!("unexpected project call") }
    async fn project_worktree(&self, _: &str, _: &crate::project_adapter::ProjectWorktreeRequest) -> Result<crate::project_adapter::ProjectWorktreeResult, InspectionError> { panic!("unexpected project call") }
    async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<crate::project_adapter::WorktreeStatus, InspectionError> { panic!("unexpected project call") }
    async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { panic!("unexpected project call") }
    async fn project_remove_worktree(&self, _: &str, _: &crate::project_adapter::ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { panic!("unexpected project call") }
}

#[async_trait::async_trait]
impl super::super::herdr::OrchestrationHerdr for NoEffectsAdapter {
    async fn runtime(&self, _: &str) -> Result<super::super::herdr::RuntimeView, InspectionError> { panic!("unexpected Herdr call") }
    async fn create_agent_tab(&self, _: &str, _: &super::super::herdr::AgentTabRequest) -> Result<RunLocation, InspectionError> { panic!("unexpected Herdr call") }
    async fn pane_process_info(&self, _: &str, _: &str, _: &str) -> Result<super::super::herdr::PaneProcessInfo, InspectionError> { panic!("unexpected Herdr call") }
    async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { panic!("unexpected Herdr call") }
}

struct Fixture {
    directory: PathBuf,
    repository: PathBuf,
    target: PathBuf,
    worker_path: PathBuf,
    service: OrchestrationService,
    run: Run,
    identity: OwnedGitIdentity,
}

impl Fixture {
    async fn new(merge: bool) -> Self {
        let directory = std::env::temp_dir().join(format!("cockpit-merged-cleanup-{}", id()));
        let repository = directory.join("repository");
        let target = repository.clone();
        let root_target = directory.join("root-target");
        let worker_path = directory.join("accepted-worker");
        std::fs::create_dir_all(&repository).unwrap();
        git(&repository, &["init", "-b", "main"]);
        git(&repository, &["config", "user.name", "Cleanup Test"]);
        git(&repository, &["config", "user.email", "cleanup@example.invalid"]);
        std::fs::write(repository.join("tracked"), "base\n").unwrap();
        git(&repository, &["add", "tracked"]);
        git(&repository, &["commit", "-m", "base"]);
        let base = git(&repository, &["rev-parse", "HEAD"]).trim().to_owned();
        git(&repository, &["worktree", "add", "-b", "root-target", root_target.to_str().unwrap()]);
        git(&repository, &["worktree", "add", "-b", "accepted-worker", worker_path.to_str().unwrap()]);
        // Capture original creation pins before the worker produces its output.
        let operation = id();
        let identity = capture_owned_git_identity(&operation, repository.to_str().unwrap(),
            worker_path.to_str().unwrap(), Some("refs/heads/accepted-worker"), TIMEOUT_MS, OUTPUT_BYTES).await.unwrap();
        std::fs::write(worker_path.join("tracked"), "accepted output\n").unwrap();
        git(&worker_path, &["add", "tracked"]);
        git(&worker_path, &["commit", "-m", "accepted worker output"]);
        let head = git(&worker_path, &["rev-parse", "HEAD"]).trim().to_owned();
        if merge { git(&target, &["merge", "--no-ff", "accepted-worker", "-m", "merge accepted worker"]); }

        let (project, checkout) = project_identity::resolve_directory(&worker_path, None, None).await.unwrap();
        let (root_project, root_checkout) = project_identity::resolve_directory(&root_target, None, None).await.unwrap();
        assert_eq!(project.key, root_project.key);
        assert_eq!(checkout.key, identity.checkout_key);
        assert_ne!(identity.creation_tip, head);
        let root_id = id();
        let session = id();
        let mut root = new_run(&root_id, &session, RunKind::Supervisor, "root".into(), &root_id, RunLineage::default(), 1);
        root.stage = RunStage::Active;
        root.project = Some(root_project);
        root.checkout = Some(root_checkout);
        let mut run = new_run(&id(), &session, RunKind::Worker, "accepted worker".into(), &root_id,
            RunLineage { parent_run_id: Some(root_id.clone()), task_id: None }, 1);
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Accepted);
        run.cleanup_policy = Some(WorkerCleanupPolicy::OwnedWhenSafe);
        run.project = Some(project.clone());
        run.checkout = Some(checkout.clone());
        run.setup = Some(SetupSummary { operation_id: Some(operation), generation: None, workspace_id: None,
            checkout_path: checkout.path.clone(), repository_id: None, branch: Some("accepted-worker".into()),
            base: Some("main".into()), base_commit: Some(base.clone()), ownership: Some(
                cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree),
            effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None });
        let result_message_id = id();
        let source = SourceOutput::Code { repository_key: project.key, checkout_key: checkout.key.clone(),
            base_commit: base, head_commit: head.clone(), commits: vec![head], task_revision: id(),
            result_message_id: result_message_id.clone(), focused_checks: Vec::new() };
        run.source_output = Some(source.clone());
        run.result = Some(Report { card: None, message_id: result_message_id, kind: ReportKind::Result,
            outcome: Some(ReportOutcome::Succeeded), summary: "accepted real Git output".into(), plan: None,
            source_output: Some(source), at: now() });
        // Arrange the service's logical release boundary, not a live scope-drain proof.
        run.attachment.as_mut().unwrap().state = AttachmentState::Revoked;
        let claim_id = id();
        run.checkout_claim_id = Some(claim_id.clone());
        let claim = claims::CheckoutClaim { claim_id, checkout, revision: 1,
            state: claims::ClaimState::Released, holder: None, transition_operation_id: None };
        let store = OrchestrationStore::open(&directory.join("store")).unwrap();
        let (revision, _) = watch::channel(0);
        let service = OrchestrationService { store, revision, notifications: std::sync::Mutex::new(None),
            launch_config: Default::default() };
        {
            let locked = service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.extend([root, run.clone()]);
            state.checkout_claims.push(claim);
            insert_resource(&mut state.owned_resources, &run,
                OwnedResourceIdentity::Worktree { identity: identity.clone() }).unwrap();
            insert_resource(&mut state.owned_resources, &run,
                OwnedResourceIdentity::Branch { identity: identity.clone() }).unwrap();
            locked.save(&mut state).unwrap();
        }
        Self { directory, repository, target, worker_path, service, run, identity }
    }

    fn state(&self) -> OrchestrationState { self.service.store.lock().unwrap().read().unwrap() }

    fn mutate(&self, change: impl FnOnce(&mut OrchestrationState)) {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        change(&mut state);
        locked.save(&mut state).unwrap();
    }

    fn resource(&self, kind: OwnedResourceKind) -> OwnedResourceRecord {
        let state = self.state();
        let mut resource = state.owned_resources.iter().find(|resource| resource.kind == kind).unwrap().clone();
        resource.consumers = OrchestrationService::consumers_unresolved(&state, &resource);
        resource
    }

    fn assert_git_preserved(&self) {
        assert!(self.worker_path.is_dir());
        let SourceOutput::Code { head_commit, .. } = self.run.source_output.as_ref().unwrap() else { unreachable!() };
        assert_eq!(git(&self.repository, &["rev-parse", "refs/heads/accepted-worker"]).trim(), head_commit);
        assert!(self.state().owned_resources.iter().all(|resource| resource.cleanup.state != CleanupState::Removed));
        assert!(matches!(self.state().runs[1].integration, Integration::Pending));
    }

    async fn remove(&self, kind: OwnedResourceKind) {
        let state = self.state();
        let proof = merged_proof(&state, &self.run).await.unwrap().expect("fresh accepted containment");
        assert!(proof.matches_state(&state, &self.run));
        proof.reprove().await.unwrap();
        let head = crate::project_teardown::cleanup_owned_head(&self.identity, &self.run, Some(&proof)).unwrap();
        let resource = self.resource(kind);
        let intent = self.service.begin_cleanup_effect(&resource, &self.run, Some(&proof)).unwrap().unwrap();
        assert_eq!(self.resource(kind).cleanup.effects.last().unwrap().state, CleanupEffectState::Intent);
        // Reprove immediately adjacent to the real non-force effect.
        proof.reprove().await.unwrap();
        let removed = match kind {
            OwnedResourceKind::Worktree => remove_owned_worktree(&self.identity, head, TIMEOUT_MS, OUTPUT_BYTES).await.unwrap(),
            OwnedResourceKind::Branch => remove_owned_branch(&self.identity, head, TIMEOUT_MS, OUTPUT_BYTES).await.unwrap(),
            _ => unreachable!(),
        };
        assert!(removed);
        self.service.finish_cleanup_effect(&intent, CleanupEffectOutcome { state: CleanupEffectState::Applied,
            evidence: Some(proof.evidence()), reason: None, effects: Vec::new() }).unwrap();
        let saved = self.resource(kind);
        assert_eq!(saved.cleanup.state, CleanupState::Removed);
        assert_eq!(saved.cleanup.effects.last().unwrap().state, CleanupEffectState::Applied);
        assert_eq!(saved.cleanup.effects.last().unwrap().evidence.as_deref(), Some(proof.evidence().as_str()));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.directory); }
}

#[tokio::test]
async fn scope_hold_stays_stable_until_drain_then_rechecks_integration() {
    let fixture = Fixture::new(false).await;
    fixture.mutate(|state| state.checkout_claims[0].state = claims::ClaimState::Held);
    let herdr = NoEffectsAdapter;
    fixture.service.cleanup_worker_scopes(&herdr).await.unwrap();
    let why = fixture.state().owned_resources[0].cleanup.reason.clone().unwrap();
    assert_eq!(why.code, "containment_unproven");
    let projects = ProjectService::new(
        cockpit_protocol::projects::ProjectConfiguration::for_tests(&fixture.directory),
        std::sync::Arc::new(NoEffectsAdapter),
    ).unwrap();
    fixture.service.cleanup_owned_resources(&projects).await.unwrap();
    let stable_revision = fixture.state().revision;
    for _ in 0..3 {
        fixture.service.cleanup_worker_scopes(&herdr).await.unwrap();
        fixture.service.cleanup_owned_resources(&projects).await.unwrap();
        let state = fixture.state();
        assert_eq!(state.revision, stable_revision, "unchanged cleanup must not wake its own scheduler");
        for resource in &state.owned_resources {
            assert_eq!(resource.cleanup.state, CleanupState::Kept);
            assert_eq!(resource.cleanup.reason.as_ref(), Some(&why));
            assert!(resource.cleanup.effects.is_empty());
        }
    }
    // A failed fresh Git inspection must not replace the stronger writer hold
    // and cause the two cleanup passes to rewrite each other indefinitely.
    let git_dir = fixture.repository.join(".git");
    let unavailable = fixture.directory.join("temporarily-unavailable-git");
    std::fs::rename(&git_dir, &unavailable).unwrap();
    assert!(merged_proof(&fixture.state(), &fixture.run).await.is_err());
    fixture.service.cleanup_worker_scopes(&herdr).await.unwrap();
    fixture.service.cleanup_owned_resources(&projects).await.unwrap();
    let held = fixture.state();
    assert_eq!(held.revision, stable_revision);
    assert!(held.owned_resources.iter().all(|resource|
        resource.cleanup.state == CleanupState::Kept && resource.cleanup.reason.as_ref() == Some(&why)));
    std::fs::rename(&unavailable, &git_dir).unwrap();
    // A same-revision external update must not leave the negative hold cached.
    let stale_snapshot = fixture.service.store.lock().unwrap().read_shared().unwrap();
    let mut released = fixture.state();
    released.checkout_claims[0].state = claims::ClaimState::Released;
    {
        let _locked = fixture.service.store.lock().unwrap();
        std::fs::write(fixture.service.base().join("state.json"), serde_json::to_vec(&released).unwrap()).unwrap();
    }
    assert!(!fixture.service.reconcile_accepted_integrations_from_snapshot(&stale_snapshot).await.unwrap());
    assert!(!ref_cleanup_noop(&released), "released claim must invalidate the negative hold");
    fixture.service.cleanup_owned_resources(&projects).await.unwrap();
    let state = fixture.state();
    assert!(state.revision > stable_revision);
    for resource in &state.owned_resources {
        assert_eq!(resource.cleanup.state, CleanupState::WaitingIntegration);
        assert_eq!(resource.cleanup.reason.as_ref().unwrap().code, "integration_pending");
        assert!(resource.cleanup.effects.is_empty());
    }
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn previous_boot_does_not_replace_pending_claim_scope_hold() {
    let Some(machine) = crate::process_identity::machine_id() else { return };
    let Some(boot) = crate::process_identity::kernel_boot_id() else { return };
    for close_reason in [CloseReason::Accepted, CloseReason::Superseded] {
        let fixture = Fixture::new(false).await;
        fixture.mutate(|state| {
            state.checkout_claims[0].state = claims::ClaimState::Held;
            let run = &mut state.runs[1];
            run.close_reason = Some(close_reason);
            let attachment = run.attachment.as_mut().unwrap();
            attachment.machine_id = Some(machine.clone());
            attachment.process = Some(cockpit_protocol::orchestration::NativeProcessIdentity {
                pid: 42, start_ticks: 7, kernel_boot_id: Some(format!("previous-{boot}")),
            });
            attachment.process_scope = None;
        });
        let herdr = NoEffectsAdapter;
        let projects = ProjectService::new(
            cockpit_protocol::projects::ProjectConfiguration::for_tests(&fixture.directory),
            std::sync::Arc::new(NoEffectsAdapter),
        ).unwrap();
        fixture.service.cleanup_worker_scopes(&herdr).await.unwrap();
        let held = fixture.state();
        let why = held.owned_resources[0].cleanup.reason.clone().unwrap();
        assert_eq!(why.code, "containment_unproven");
        fixture.service.cleanup_owned_resources(&projects).await.unwrap();
        let settled = fixture.state();
        assert!(settled.owned_resources.iter().all(|resource|
            resource.cleanup.state == CleanupState::Kept && resource.cleanup.reason.as_ref() == Some(&why)),
            "the first resource assessment must preserve the pending-claim hold: {:?}", settled.owned_resources);
        let revision = settled.revision;
        for _ in 0..3 {
            fixture.service.cleanup_owned_resources(&projects).await.unwrap();
            fixture.service.cleanup_worker_scopes(&herdr).await.unwrap();
            let state = fixture.state();
            assert_eq!(state.revision, revision, "competing assessments must retain the pending-claim hold");
            assert_eq!(state.checkout_claims[0].state, claims::ClaimState::Held);
            assert!(state.owned_resources.iter().all(|resource|
                resource.cleanup.state == CleanupState::Kept
                    && resource.cleanup.reason.as_ref() == Some(&why)
                    && resource.cleanup.effects.is_empty()));
        }
        // A separately established release re-enables the ordinary assessment.
        fixture.mutate(|state| state.checkout_claims[0].state = claims::ClaimState::Released);
        fixture.service.cleanup_owned_resources(&projects).await.unwrap();
        let state = fixture.state();
        assert!(state.revision > revision);
        assert!(state.owned_resources.iter().all(|resource|
            resource.cleanup.reason.as_ref() != Some(&why) && resource.cleanup.effects.is_empty()));
        assert!(fixture.worker_path.is_dir());
    }
}

#[tokio::test]
async fn accepted_no_ff_merge_removes_worktree_then_branch_without_integration_record() {
    let fixture = Fixture::new(true).await;
    let before = fixture.state();
    let original_source = before.runs[1].source_output.clone();
    let original_result = serde_json::to_value(&before.runs[1].result).unwrap();
    assert!(crate::project_teardown::cleanup_owned_head(&fixture.identity, &fixture.run, None).is_err());
    let target = git(&fixture.target, &["rev-parse", "HEAD"]).trim().to_owned();
    let parents = git(&fixture.target, &["rev-list", "--parents", "-n", "1", &target]);
    assert_eq!(parents.split_whitespace().count(), 3, "primary target is a no-ff merge");
    fixture.remove(OwnedResourceKind::Worktree).await;
    assert!(!fixture.worker_path.exists());
    assert_eq!(fixture.resource(OwnedResourceKind::Branch).cleanup.state, CleanupState::Pending);
    fixture.remove(OwnedResourceKind::Branch).await;
    assert!(!crate::space_git::git_output(&fixture.repository,
        &["show-ref", "--verify", "--quiet", "refs/heads/accepted-worker"]).await.unwrap().status.success());
    let saved = fixture.state();
    assert!(matches!(saved.runs[1].integration, Integration::Pending));
    assert_eq!(saved.runs[1].source_output, original_source);
    assert_eq!(serde_json::to_value(&saved.runs[1].result).unwrap(), original_result);
    assert!(saved.owned_resources.iter().all(|resource| resource.integration_result_id.is_none()));
    for resource in &saved.owned_resources {
        let (OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity }) = &resource.identity else { unreachable!() };
        assert_eq!(identity, &fixture.identity, "removal never rewrites creation pins");
    }
}

#[tokio::test]
async fn unmerged_accepted_head_has_no_proof_and_waits_for_integration() {
    let fixture = Fixture::new(false).await;
    assert!(merged_proof(&fixture.state(), &fixture.run).await.unwrap().is_none());
    for kind in [OwnedResourceKind::Worktree, OwnedResourceKind::Branch] {
        let resource = fixture.resource(kind);
        fixture.service.save_evaluation(&resource, CleanupEvaluation::Waiting(reason(
            CleanupReasonClass::Reconcilable, "integration_pending", "Accepted head is not contained in actual root target"))).unwrap();
        let waiting = fixture.resource(kind);
        assert_eq!(waiting.cleanup.state, CleanupState::WaitingIntegration);
        assert!(waiting.cleanup.effects.is_empty());
        assert!(fixture.service.begin_cleanup_effect(&waiting, &fixture.run, None).unwrap().is_none());
    }
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn cherry_picked_output_removes_worktree_and_branch() {
    let fixture = Fixture::new(false).await;
    let SourceOutput::Code { head_commit, .. } = fixture.run.source_output.as_ref().unwrap() else { unreachable!() };
    // Different parents ensure the cherry-pick cannot reproduce the accepted SHA.
    git(&fixture.target, &["commit", "--allow-empty", "-m", "diverge primary"]);
    git(&fixture.target, &["cherry-pick", head_commit]);
    let proof = merged_proof(&fixture.state(), &fixture.run).await.unwrap().unwrap();
    assert!(proof.evidence().contains("patch-equivalent"));
    proof.reprove().await.unwrap();
    fixture.remove(OwnedResourceKind::Worktree).await;
    fixture.remove(OwnedResourceKind::Branch).await;
}

#[tokio::test]
async fn cherry_pick_reproof_fences_target_movement() {
    let fixture = Fixture::new(false).await;
    let SourceOutput::Code { head_commit, .. } = fixture.run.source_output.as_ref().unwrap() else { unreachable!() };
    git(&fixture.target, &["commit", "--allow-empty", "-m", "diverge primary"]);
    git(&fixture.target, &["cherry-pick", head_commit]);
    let proof = merged_proof(&fixture.state(), &fixture.run).await.unwrap().unwrap();
    git(&fixture.target, &["commit", "--allow-empty", "-m", "advance primary"]);
    assert_eq!(proof.reprove().await.unwrap_err().code, "merged_target_changed");
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn partial_cherry_pick_waits_for_integration() {
    let mut fixture = Fixture::new(false).await;
    let SourceOutput::Code { head_commit: first, .. } = fixture.run.source_output.as_ref().unwrap() else { unreachable!() };
    let first = first.clone();
    std::fs::write(fixture.worker_path.join("second"), "second patch\n").unwrap();
    git(&fixture.worker_path, &["add", "second"]);
    git(&fixture.worker_path, &["commit", "-m", "second output"]);
    let second = git(&fixture.worker_path, &["rev-parse", "HEAD"]).trim().to_owned();
    let SourceOutput::Code { head_commit, commits, .. } = fixture.run.source_output.as_mut().unwrap() else { unreachable!() };
    *head_commit = second.clone();
    *commits = vec![first.clone(), second];
    fixture.run.result.as_mut().unwrap().source_output = fixture.run.source_output.clone();
    fixture.mutate(|state| state.runs[1] = fixture.run.clone());
    git(&fixture.target, &["commit", "--allow-empty", "-m", "diverge primary"]);
    git(&fixture.target, &["cherry-pick", &first]);
    assert!(merged_proof(&fixture.state(), &fixture.run).await.unwrap().is_none());
    let resource = fixture.resource(OwnedResourceKind::Worktree);
    fixture.service.save_evaluation(&resource, CleanupEvaluation::Waiting(reason(
        CleanupReasonClass::Reconcilable, "integration_pending", "Partial integration"))).unwrap();
    assert_eq!(fixture.resource(OwnedResourceKind::Worktree).cleanup.state, CleanupState::WaitingIntegration);
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn bare_repository_without_primary_checkout_waits() {
    let fixture = Fixture::new(false).await;
    let bare = fixture.directory.join("bare.git");
    git(&fixture.directory, &["init", "--bare", bare.to_str().unwrap()]);
    let mut run = fixture.run.clone();
    run.checkout.as_mut().unwrap().git.as_mut().unwrap().common_dir = bare.to_str().unwrap().into();
    assert!(merged_proof(&fixture.state(), &run).await.unwrap().is_none());
    let resource = fixture.resource(OwnedResourceKind::Worktree);
    fixture.service.save_evaluation(&resource, CleanupEvaluation::Waiting(reason(
        CleanupReasonClass::Reconcilable, "integration_pending", "No primary checkout"))).unwrap();
    assert_eq!(fixture.resource(OwnedResourceKind::Worktree).cleanup.state, CleanupState::WaitingIntegration);
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn merge_in_another_checkout_does_not_prove_primary_head() {
    let fixture = Fixture::new(false).await;
    let root = fixture.directory.join("root-target");
    git(&root, &["merge", "--no-ff", "accepted-worker", "-m", "merge outside primary"]);
    assert!(merged_proof(&fixture.state(), &fixture.run).await.unwrap().is_none());
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn supervisor_checkout_is_not_the_project_target() {
    let fixture = Fixture::new(true).await;
    let mut state = fixture.state();
    state.runs[0].checkout = None;
    state.runs[0].project = None;
    assert!(merged_proof(&state, &fixture.run).await.unwrap().is_some());
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn root_advance_then_reset_fences_adjacent_merged_reproof_without_removal() {
    let fixture = Fixture::new(true).await;
    let merged = git(&fixture.target, &["rev-parse", "HEAD"]).trim().to_owned();
    let proof = merged_proof(&fixture.state(), &fixture.run).await.unwrap().unwrap();
    git(&fixture.target, &["commit", "--allow-empty", "-m", "root advance"]);
    assert_eq!(proof.reprove().await.unwrap_err().code, "merged_target_changed");
    let advanced = merged_proof(&fixture.state(), &fixture.run).await.unwrap().unwrap();
    let resource = fixture.resource(OwnedResourceKind::Worktree);
    let intent = fixture.service.begin_cleanup_effect(&resource, &fixture.run, Some(&advanced)).unwrap().unwrap();
    git(&fixture.target, &["reset", "--hard", &merged]);
    assert_eq!(advanced.reprove().await.unwrap_err().code, "merged_target_changed");
    assert_eq!(fixture.resource(OwnedResourceKind::Worktree).cleanup.effects.last().unwrap().effect_id,
        intent.cleanup.effects.last().unwrap().effect_id);
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn merged_dirty_or_untracked_worker_is_refused_and_preserved() {
    for tracked in [true, false] {
        let fixture = Fixture::new(true).await;
        let path = fixture.worker_path.join(if tracked { "tracked" } else { "untracked-output" });
        std::fs::write(&path, "preserve local output\n").unwrap();
        let proof = merged_proof(&fixture.state(), &fixture.run).await.unwrap().unwrap();
        let head = crate::project_teardown::cleanup_owned_head(&fixture.identity, &fixture.run, Some(&proof)).unwrap();
        let resource = fixture.resource(OwnedResourceKind::Worktree);
        let intent = fixture.service.begin_cleanup_effect(&resource, &fixture.run, Some(&proof)).unwrap().unwrap();
        proof.reprove().await.unwrap();
        let refused = remove_owned_worktree(&fixture.identity, head, TIMEOUT_MS, OUTPUT_BYTES).await.unwrap_err();
        fixture.service.finish_cleanup_effect(&intent, CleanupEffectOutcome { state: CleanupEffectState::Refused,
            evidence: None, reason: Some(reason(CleanupReasonClass::Reconcilable, &refused.code, &refused.message)),
            effects: Vec::new() }).unwrap();
        let retained = fixture.resource(OwnedResourceKind::Worktree);
        assert!(matches!(retained.cleanup.state, CleanupState::Waiting | CleanupState::Kept));
        assert!(retained.cleanup.reason.is_some());
        assert_eq!(fixture.resource(OwnedResourceKind::Worktree).cleanup.effects.last().unwrap().state, CleanupEffectState::Refused);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "preserve local output\n");
        assert!(remove_owned_branch(&fixture.identity, head, TIMEOUT_MS, OUTPUT_BYTES).await.is_err());
        fixture.assert_git_preserved();
    }
}

#[tokio::test]
async fn merged_proof_cannot_override_unreleased_claim_or_concurrent_writer() {
    for concurrent_writer in [false, true] {
        let fixture = Fixture::new(true).await;
        let proof = merged_proof(&fixture.state(), &fixture.run).await.unwrap().unwrap();
        let worktree = fixture.resource(OwnedResourceKind::Worktree);
        let branch = fixture.resource(OwnedResourceKind::Branch);
        fixture.mutate(|state| {
            if concurrent_writer {
                let writer_id = id();
                let mut writer = new_run(&writer_id, &fixture.run.session_id, RunKind::Worker, "concurrent writer".into(),
                    &fixture.run.root_id, RunLineage { parent_run_id: Some(fixture.run.root_id.clone()), task_id: None }, 1);
                writer.stage = RunStage::Active;
                writer.checkout = fixture.run.checkout.clone();
                state.runs.push(writer);
            } else {
                state.checkout_claims[0].state = claims::ClaimState::Held;
            }
        });
        let before = std::fs::read(fixture.service.base().join("state.json")).unwrap();
        for resource in [&worktree, &branch] {
            assert!(fixture.service.begin_cleanup_effect(resource, &fixture.run, Some(&proof)).unwrap().is_none());
        }
        assert_eq!(std::fs::read(fixture.service.base().join("state.json")).unwrap(), before);
        fixture.assert_git_preserved();
    }
}

#[tokio::test]
async fn changed_accepted_result_or_root_session_fences_merged_proof() {
    for changed in 0..3 {
        let fixture = Fixture::new(true).await;
        let proof = merged_proof(&fixture.state(), &fixture.run).await.unwrap().unwrap();
        let resource = fixture.resource(OwnedResourceKind::Worktree);
        fixture.mutate(|state| match changed {
            0 => {
                let result = id();
                state.runs[1].result.as_mut().unwrap().message_id = result.clone();
                let SourceOutput::Code { result_message_id, .. } = state.runs[1].source_output.as_mut().unwrap() else { unreachable!() };
                *result_message_id = result;
                state.runs[1].result.as_mut().unwrap().source_output = state.runs[1].source_output.clone();
            }
            1 => {
                let SourceOutput::Code { task_revision, .. } = state.runs[1].source_output.as_mut().unwrap() else { unreachable!() };
                *task_revision = id();
                state.runs[1].result.as_mut().unwrap().source_output = state.runs[1].source_output.clone();
            }
            _ => state.runs[0].session_id = id(),
        });
        let state = fixture.state();
        assert!(!proof.matches_state(&state, &state.runs[1]));
        assert!(fixture.service.begin_cleanup_effect(&resource, &state.runs[1], Some(&proof)).unwrap().is_none());
        assert!(fixture.resource(OwnedResourceKind::Worktree).cleanup.effects.is_empty());
        fixture.assert_git_preserved();
    }
}

#[tokio::test]
async fn merged_head_requires_exact_successful_accepted_result() {
    let fixture = Fixture::new(true).await;
    for changed in 0..4 {
        let mut state = fixture.state();
        let run = &mut state.runs[1];
        match changed {
            0 => run.result.as_mut().unwrap().message_id = id(),
            1 => run.result.as_mut().unwrap().outcome = Some(ReportOutcome::Failed),
            2 => run.result.as_mut().unwrap().source_output = None,
            _ => run.close_reason = Some(CloseReason::Superseded),
        }
        assert!(merged_proof(&state, &state.runs[1]).await.unwrap().is_none());
    }
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn changed_retained_directory_only_skips_an_existing_exact_conflict() {
    let fixture = Fixture::new(true).await;
    fixture.mutate(|state| {
        for resource in &mut state.owned_resources {
            resource.cleanup.state = CleanupState::Kept;
            resource.cleanup.reason = Some(reason(CleanupReasonClass::UserWork, "owned_git_conflict",
                crate::project_adapter::owned_git::DIRECTORY_CHANGED));
        }
    });
    let projects = ProjectService::new(
        cockpit_protocol::projects::ProjectConfiguration::for_tests(&fixture.directory),
        std::sync::Arc::new(NoEffectsAdapter),
    ).unwrap();
    let saved = fixture.directory.join("saved-repository");
    std::fs::rename(&fixture.repository, &saved).unwrap();
    // Missing or inaccessible identity is uncertain, not a negative proof.
    assert_eq!(cleanup_evaluation_candidates(&fixture.state()).len(), 2);
    std::fs::create_dir(&fixture.repository).unwrap();
    let state = fixture.state();
    assert!(cleanup_evaluation_candidates(&state).is_empty());
    assert!(!ref_cleanup_noop(&state), "unreported holds require the normal observer");
    fixture.service.cleanup_owned_resources(&projects).await.unwrap();
    let reported = fixture.state();
    assert!(ref_cleanup_noop(&reported));
    let mut with_receipt = reported.clone();
    let source = fixture.run.source_output.clone().unwrap();
    let SourceOutput::Code { repository_key, result_message_id, task_revision, head_commit, .. } = &source else { unreachable!() };
    with_receipt.runs[0].integration = Integration::Integrated {
        operation_id: id(), accepted_result_message_id: result_message_id.clone(),
        accepted_task_revision: task_revision.clone(), source: Box::new(source.clone()),
        target_repository_key: repository_key.clone(), target_ref: "refs/heads/main".into(),
        target_commit: head_commit.clone(), method: cockpit_protocol::orchestration::IntegrationMethod::Ancestor,
        mappings: Vec::new().into(), checks: Vec::new(), integrator_run_id: fixture.run.root_id.clone(), verified_at: now(),
    };
    assert!(!ref_cleanup_noop(&with_receipt), "a target receipt requires ref-sensitive reconciliation");
    assert_eq!(reported.owned_resources, state.owned_resources);
    // The first pass can publish the existing holds to the root inbox.
    fixture.service.cleanup_owned_resources(&projects).await.unwrap();
    assert_eq!(fixture.state().revision, reported.revision);
    // Even with the same revision, freshly read bytes can reveal that a hold
    // was not delivered. The negative fast path must publish it again.
    let mut edited = fixture.state();
    let old_count = edited.messages.len();
    edited.messages.retain(|message| !message.message_id.starts_with("cleanup:"));
    assert!(edited.messages.len() < old_count);
    let same_revision = edited.revision;
    {
        let _locked = fixture.service.store.lock().unwrap();
        std::fs::write(fixture.service.base().join("state.json"), serde_json::to_vec(&edited).unwrap()).unwrap();
    }
    fixture.service.cleanup_owned_resources(&projects).await.unwrap();
    let restored = fixture.state();
    assert!(restored.revision > same_revision);
    assert_eq!(restored.messages.len(), old_count);
    assert_eq!(restored.owned_resources, edited.owned_resources);
    for resource in &state.owned_resources {
        assert!(fixture.service.begin_cleanup_effect(resource, &fixture.run, None).unwrap().is_none());
    }
    let mut uncertain = state.clone();
    let resource = &mut uncertain.owned_resources[0];
    resource.cleanup.effects.push(CleanupEffectReceipt {
        effect_id: id(), kind: resource.kind, identity_digest: resource.resource_id.clone(),
        state: CleanupEffectState::Unknown, evidence: None, updated_at: now(),
    });
    assert_eq!(cleanup_evaluation_candidates(&uncertain).len(), 1);
    std::fs::remove_dir(&fixture.repository).unwrap();
    std::fs::rename(&saved, &fixture.repository).unwrap();
    assert_eq!(cleanup_evaluation_candidates(&fixture.state()).len(), 2);
    assert!(!ref_cleanup_noop(&fixture.state()), "restored physical identity must be reassessed");
    fixture.service.cleanup_owned_resources(&projects).await.unwrap();
    assert!(fixture.state().owned_resources.iter().all(|resource|
        resource.cleanup.reason.as_ref().is_none_or(|why| why.code != "owned_git_conflict")));
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn linked_root_head_watch_retires_only_after_both_git_resources_are_removed() {
    let fixture = Fixture::new(true).await;
    let state = fixture.state();
    let root_git = state.runs[0].checkout.as_ref().unwrap().git.as_ref().unwrap();
    let root_git_dir = PathBuf::from(&root_git.worktree_git_dir);
    let common_dir = PathBuf::from(&root_git.common_dir);
    assert_ne!(root_git_dir, common_dir);
    assert!(Path::new(&root_git_dir).join("HEAD").is_file());
    let watched = fixture.service.cleanup_ref_watch_paths().unwrap();
    assert!(watched.contains(&root_git_dir), "linked root HEAD parent must be watched");
    assert!(watched.contains(&common_dir));
    fixture.remove(OwnedResourceKind::Worktree).await;
    let watched = fixture.service.cleanup_ref_watch_paths().unwrap();
    assert!(watched.contains(&root_git_dir));
    assert!(watched.contains(&common_dir));
    fixture.remove(OwnedResourceKind::Branch).await;
    assert!(fixture.service.cleanup_ref_watch_paths().unwrap().is_empty());
    assert!(matches!(fixture.state().runs[1].integration, Integration::Pending));
}
