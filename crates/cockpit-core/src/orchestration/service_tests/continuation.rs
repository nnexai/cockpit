use super::*;
use super::super::mutate::continuation::ContinueIntent;
use super::super::store::{DurableOperationKind, DurableOperationRecord};

fn action(root: &str, task: &Task) -> OrchestrationAction {
    OrchestrationAction::TaskContinue { operation_id: id(), root_id: root.into(), task_id: task.task_id.clone(),
        expected_task_revision: task.task_revision.clone(), text: "Address the follow-up".into() }
}

#[test]
fn reported_continuation_reopens_worker_and_replays() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run, worker) = fixture.working(&root);
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    let request = action(&root, &task);
    let first = fixture.operator(request.clone());
    assert!(matches!(first.result, OrchestrationActionResult::TaskContinued { route: ContinueRoute::SameWorker { .. }, duplicate: false, .. }));
    let updated = fixture.task(&root, &task.task_id);
    assert_ne!(updated.task_revision, task.task_revision);
    assert!(updated.description.contains("Address the follow-up"));
    assert_eq!(fixture.run(&run).stage, RunStage::Working);
    assert!(fixture.run(&run).result.is_none());
    assert!(fixture.state().messages.iter().any(|m| m.to_run_id == run && m.kind == MessageKind::Answer && m.text == "Address the follow-up"));
    assert!(matches!(fixture.operator(request).result, OrchestrationActionResult::TaskContinued { duplicate: true, .. }));
}

#[test]
fn accepted_continuation_unchecks_and_schedules_successor() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run, worker) = fixture.working(&root);
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    fixture.operator(OrchestrationAction::Accept { run_id: run.clone(), expected_task_revision: task.task_revision.clone() });
    let accepted = fixture.task(&root, &task.task_id);
    fixture.seed_run(&run, |attempt| {
        let result_id = attempt.result.as_ref().unwrap().message_id.clone();
        attempt.target = Some(DispatchTarget::SpaceWorktree { workspace_id: "project-space".into(), branch: Some("old".into()), base_ref: None });
        attempt.source_output = Some(SourceOutput::Code { repository_key: "repo".into(), checkout_key: "checkout".into(),
            base_commit: "base".into(), head_commit: "accepted-head".into(), commits: vec!["accepted-head".into()],
            task_revision: task.task_revision.clone(), result_message_id: result_id, focused_checks: Vec::new() });
    });
    assert!(accepted.checked);
    assert!(matches!(fixture.operator(action(&root, &accepted)).result, OrchestrationActionResult::TaskContinued { route: ContinueRoute::Successor, .. }));
    let updated = fixture.task(&root, &task.task_id);
    assert!(!updated.checked);
    let state = fixture.state();
    let owner = state.task_ownership.iter().find(|o| o.root_id == root && o.task_id == task.task_id).unwrap();
    assert!(owner.owner.is_none());
    assert_eq!(owner.reopened_after_attempt, Some(fixture.run(&run).attempt));
    assert!(owner.schedule.is_some());
    assert!(!super::super::mutate::tasks::accepted_blocks(&state, &root, &task.task_id));
    assert!(matches!(&owner.schedule.as_ref().unwrap().target,
        Some(DispatchTarget::SpaceWorktree { workspace_id, branch: None, base_ref: Some(head) })
            if workspace_id == "project-space" && head == "accepted-head"));
    drop(state);
    fixture.operator(OrchestrationAction::TaskUpdate { root_id: root, task_id: task.task_id, expected_task_revision: updated.task_revision,
        title: Some("Continued task".into()), description: None });
}

#[test]
fn accepted_continuation_is_dispatchable_at_next_generation() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let (task, run, worker) = fixture.working(&root);
    fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
    fixture.operator(OrchestrationAction::Accept { run_id: run.clone(), expected_task_revision: task.task_revision.clone() });
    let generation = fixture.run(&run).assignment_generation.unwrap();
    let accepted = fixture.task(&root, &task.task_id);
    fixture.operator(action(&root, &accepted));
    let candidate = fixture.service.schedule_candidates(4).unwrap().into_iter().find(|candidate| candidate.task_id == task.task_id).unwrap();
    let successor = fixture.service.scheduled_start(&candidate, &ResolvedMutationInput { project: Some(fixture.project()), ..Default::default() }).unwrap().unwrap();
    assert_eq!(successor.attempt, fixture.run(&run).attempt + 1);
    assert_eq!(fixture.run(&successor.run_id).assignment_generation, Some(generation + 1));
}

#[test]
fn continuation_recovers_before_and_after_publication_and_retains_conflicts() {
    for scenario in 0..3 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (task, run, worker) = fixture.working(&root);
        fixture.apply(&worker, fixture.successful_result(&worker)).unwrap();
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let document = locked.tasks(&root).unwrap();
        let description = format!("{}\n\n**Follow-up (2026-01-01):** More", task.description);
        let prepared = document.prepare_continue(&task.task_id, &task.task_revision, &description).unwrap();
        let intended = prepared.task.task_revision.clone();
        let operation_id = id();
        let route = ContinueRoute::SameWorker { run_id: run.clone() };
        state.operations.push(DurableOperationRecord { operation_id: operation_id.clone(), actor_run_id: None, principal_key: "operator".into(), root_id: root.clone(),
            kind: DurableOperationKind::TaskContinue, request_digest: "test".into(), run_id: None, resume_operation_id: None,
            result: Some(OrchestrationActionResult::TaskContinued { operation_id: operation_id.clone(), task_id: task.task_id.clone(), task_revision: intended.clone(), route: route.clone(), duplicate: false }),
            effect_receipts: Vec::new(), created_at: now(), updated_at: now() });
        state.continue_intents.push(ContinueIntent { operation_id, principal_key: "operator".into(), root_id: root.clone(), task_id: task.task_id.clone(), run_id: run.clone(),
            expected_task_revision: task.task_revision.clone(), expected_doc_revision: document.doc_revision.clone(), intended_task_revision: intended,
            description, text: "More".into(), from: ActorRef::Operator, route, state: IntentState::Pending });
        locked.save(&mut state).unwrap();
        if scenario == 1 { prepared.publish().unwrap(); }
        else if scenario == 2 { document.update(&task.task_id, &task.task_revision, Some("External edit"), None).unwrap(); }
        drop(document);
        drop(locked);
        fixture.service.recover_intents().unwrap();
        if scenario == 2 {
            assert_eq!(fixture.state().continue_intents[0].state, IntentState::Conflict);
            assert_eq!(fixture.run(&run).stage, RunStage::Reported);
            assert_eq!(fixture.task(&root, &task.task_id).title, "External edit");
        } else {
            assert!(fixture.state().continue_intents.is_empty());
            assert_eq!(fixture.run(&run).stage, RunStage::Working);
            assert!(fixture.task(&root, &task.task_id).description.contains("More"));
        }
    }
}
