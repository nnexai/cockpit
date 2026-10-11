use super::*;
use std::path::PathBuf;

fn worker() -> Run {
    Run {
        session_id: "cleanup-test-session".into(), brief: String::new(), run_id: id(),
        kind: RunKind::Worker, label: "worker".into(), root_id: id(), parent_run_id: Some(id()),
        task_id: None, attempt: 1, task_revision_at_start: None, stage: RunStage::Closed,
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
        let service = OrchestrationService { store, revision };
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
