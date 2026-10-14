use super::*;
use super::merged::merged_proof;
use crate::project_adapter::{capture_owned_git_identity, remove_owned_branch, remove_owned_worktree};
use crate::space_git::tests::git;
use std::path::{Path, PathBuf};

const TIMEOUT_MS: u64 = 10_000;
const OUTPUT_BYTES: usize = 128 * 1024;

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
        let target = directory.join("root-target");
        let worker_path = directory.join("accepted-worker");
        std::fs::create_dir_all(&repository).unwrap();
        git(&repository, &["init", "-b", "main"]);
        git(&repository, &["config", "user.name", "Cleanup Test"]);
        git(&repository, &["config", "user.email", "cleanup@example.invalid"]);
        std::fs::write(repository.join("tracked"), "base\n").unwrap();
        git(&repository, &["add", "tracked"]);
        git(&repository, &["commit", "-m", "base"]);
        let base = git(&repository, &["rev-parse", "HEAD"]).trim().to_owned();
        git(&repository, &["worktree", "add", "-b", "root-target", target.to_str().unwrap()]);
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
        let (root_project, root_checkout) = project_identity::resolve_directory(&target, None, None).await.unwrap();
        assert_eq!(project.key, root_project.key);
        assert_eq!(checkout.key, identity.checkout_key);
        assert_ne!(identity.creation_tip, head);
        let root_id = id();
        let session = id();
        let mut root = new_run(&root_id, &session, RunKind::Supervisor, "root".into(), &root_id, None, None, 1);
        root.stage = RunStage::Active;
        root.project = Some(root_project);
        root.checkout = Some(root_checkout);
        let mut run = new_run(&id(), &session, RunKind::Worker, "accepted worker".into(), &root_id,
            Some(root_id.clone()), None, 1);
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
        run.result = Some(Report { message_id: result_message_id, kind: ReportKind::Result,
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
async fn accepted_no_ff_merge_removes_worktree_then_branch_without_integration_record() {
    let fixture = Fixture::new(true).await;
    let before = fixture.state();
    let original_source = before.runs[1].source_output.clone();
    let original_result = serde_json::to_value(&before.runs[1].result).unwrap();
    assert!(crate::project_teardown::cleanup_owned_head(&fixture.identity, &fixture.run, None).is_err());
    let target = git(&fixture.target, &["rev-parse", "HEAD"]).trim().to_owned();
    let parents = git(&fixture.target, &["rev-list", "--parents", "-n", "1", &target]);
    assert_eq!(parents.split_whitespace().count(), 3, "actual root target is a no-ff merge");
    // The retained primary remains unmerged; a shared common directory is not the target.
    assert_eq!(git(&fixture.repository, &["rev-parse", "HEAD"]).trim(), fixture.identity.creation_tip);
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
async fn merge_in_another_checkout_does_not_prove_actual_root_head() {
    let fixture = Fixture::new(false).await;
    git(&fixture.repository, &["merge", "--no-ff", "accepted-worker", "-m", "merge outside root"]);
    assert!(merged_proof(&fixture.state(), &fixture.run).await.unwrap().is_none());
    fixture.assert_git_preserved();
}

#[tokio::test]
async fn actual_root_same_checkout_or_foreign_repository_has_no_merged_proof() {
    let fixture = Fixture::new(true).await;
    let mut same = fixture.state();
    same.runs[0].checkout = fixture.run.checkout.clone();
    assert!(merged_proof(&same, &fixture.run).await.unwrap().is_none());
    let foreign = fixture.directory.join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    git(&foreign, &["init", "-b", "main"]);
    git(&foreign, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid",
        "commit", "--allow-empty", "-m", "foreign"]);
    let (project, checkout) = project_identity::resolve_directory(&foreign, None, None).await.unwrap();
    let mut wrong = fixture.state();
    wrong.runs[0].project = Some(project);
    wrong.runs[0].checkout = Some(checkout);
    assert!(merged_proof(&wrong, &fixture.run).await.unwrap().is_none());
    // Even a falsely reused canonical project key cannot bridge distinct Git common dirs.
    wrong.runs[0].project.as_mut().unwrap().key = fixture.run.project.as_ref().unwrap().key.clone();
    assert!(merged_proof(&wrong, &fixture.run).await.unwrap().is_none());
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
        assert_eq!(fixture.resource(OwnedResourceKind::Worktree).cleanup.state, CleanupState::Waiting);
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
                    &fixture.run.root_id, Some(fixture.run.root_id.clone()), None, 1);
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
async fn changed_accepted_result_or_root_metadata_fences_merged_proof() {
    for changed in 0..6 {
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
            2 => state.runs[0].checkout.as_mut().unwrap().key = id(),
            3 => state.runs[0].checkout.as_mut().unwrap().path = fixture.repository.to_str().unwrap().into(),
            4 => state.runs[0].project.as_mut().unwrap().key = id(),
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
