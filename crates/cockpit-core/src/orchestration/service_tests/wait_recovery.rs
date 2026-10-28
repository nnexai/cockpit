use super::*;

fn wait_request(fixture: &Fixture, timeout_ms: u32) -> OrchestrationWaitRequest {
    OrchestrationWaitRequest {
        root_id: None,
        after_revision: fixture.state().revision,
        after_tasks_token: fixture.service.store.tasks_token().unwrap(),
        timeout_ms,
    }
}

fn root_wait_request(fixture: &Fixture, root_id: &str, timeout_ms: u32) -> OrchestrationWaitRequest {
    let snapshot = fixture.service.snapshot_saved(&OrchestrationSnapshotRequest {
        session_id: SESSION.into(), root_id: Some(root_id.into()),
    }).unwrap();
    OrchestrationWaitRequest {
        root_id: Some(root_id.into()), after_revision: snapshot.revision,
        after_tasks_token: snapshot.tasks_token, timeout_ms,
    }
}

#[tokio::test]
async fn cleanup_ref_watch_delivers_external_head_packed_and_loose_ref_replacements() {
    for relative in ["HEAD", "packed-refs", "refs/heads/main"] {
        let fixture = Fixture::new();
        fixture.root();
        let common = fixture.root.join("git-common");
        std::fs::create_dir_all(common.join("refs/heads")).unwrap();
        let path = common.join(relative);
        std::fs::write(&path, "old ref").unwrap();
        let revision = fixture.state().revision;
        let mut changes = fixture.service.changes().unwrap();
        fixture.service.watch_cleanup_refs(&[common]).unwrap();
        let replacement = path.with_extension("lock");
        std::fs::write(&replacement, "new ref").unwrap();
        std::fs::rename(replacement, &path).unwrap();
        tokio::time::timeout(Duration::from_secs(1), changes.changed()).await.unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "new ref");
        assert_eq!(fixture.state().revision, revision, "External ref wake must not mutate orchestration state");
        fixture.service.watch_cleanup_refs(&[]).unwrap();
    }
}

#[tokio::test]
async fn linked_root_head_watch_ignores_index_objects_and_head_lock_churn() {
    let fixture = Fixture::new();
    fixture.root();
    let git_dir = fixture.root.join("git-common/worktrees/root");
    std::fs::create_dir_all(git_dir.join("objects")).unwrap();
    std::fs::write(git_dir.join("HEAD"), "old head").unwrap();
    let mut changes = fixture.service.changes().unwrap();
    fixture.service.watch_cleanup_refs(std::slice::from_ref(&git_dir)).unwrap();
    for relative in ["index", "HEAD.lock", "objects/object"] {
        std::fs::write(git_dir.join(relative), "unrelated churn").unwrap();
    }
    assert!(tokio::time::timeout(Duration::from_millis(100), changes.changed()).await.is_err());
    std::fs::write(git_dir.join("HEAD.lock"), "new detached head").unwrap();
    std::fs::rename(git_dir.join("HEAD.lock"), git_dir.join("HEAD")).unwrap();
    tokio::time::timeout(Duration::from_secs(1), changes.changed()).await.unwrap().unwrap();
    fixture.service.watch_cleanup_refs(&[]).unwrap();
}

#[tokio::test]
async fn cleanup_attempt_output_watch_wakes_when_published_entry_moves() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let attempt = fixture.service.base().join("attempts").join(&root);
    std::fs::create_dir_all(&attempt).unwrap();
    let output = attempt.join("published-output.txt");
    std::fs::write(&output, "preserved output").unwrap();
    let revision = fixture.state().revision;
    let mut changes = fixture.service.changes().unwrap();
    // Attempt entry watches must be installed even without Git resources.
    fixture.service.watch_cleanup_refs(&[]).unwrap();
    let published = fixture.root.join("published-output.txt");
    std::fs::rename(output, &published).unwrap();
    tokio::time::timeout(Duration::from_secs(1), changes.changed()).await.unwrap().unwrap();
    assert_eq!(std::fs::read_dir(attempt).unwrap().count(), 0);
    assert_eq!(std::fs::read_to_string(published).unwrap(), "preserved output");
    assert_eq!(fixture.state().revision, revision, "Output entry wake must not mutate orchestration state");
}

#[tokio::test(start_paused = true)]
async fn wait_unchanged_root_document_uses_snapshot_scope_and_full_deadline() {
    let fixture = Fixture::new();
    let root = fixture.root();
    fixture.create_task(&root, "Existing root document");
    let request = root_wait_request(&fixture, &root, 50);
    assert_ne!(request.after_tasks_token, fixture.service.store.tasks_token().unwrap());
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(49)).await;
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(1)).await;
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(!response.changed);
    assert_eq!(response.tasks_token, request.after_tasks_token);
    assert_eq!(tokio::time::Instant::now() - started, Duration::from_millis(50));
}

#[tokio::test]
async fn wait_scoped_root_ignores_unrelated_external_markdown_edit() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let other = fixture.root();
    fixture.create_task(&root, "Selected document");
    fixture.create_task(&other, "Unrelated document");
    let request = root_wait_request(&fixture, &root, 1_000);
    let mut events = fixture.service.changes().unwrap();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    let path = fixture.service.base().join("tasks").join(format!("{other}.md"));
    let text = std::fs::read_to_string(&path).unwrap().replace("Unrelated document", "Externally edited unrelated document");
    std::fs::write(path, text).unwrap();
    tokio::time::timeout(Duration::from_secs(1), events.changed()).await.unwrap().unwrap();
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    let response = waiting.await.unwrap();
    assert!(!response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_eq!(response.tasks_token, request.after_tasks_token);
}

#[tokio::test(start_paused = true)]
async fn durable_inbox_wait_wakes_on_message_without_reading_or_acknowledging_body() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let actor = fixture.root_actor(&root);
    let Actor::Agent(caller) = &actor else { unreachable!() };
    let after = fixture.state().messages.iter().filter(|message| message.to_run_id == root)
        .map(|message| message.seq).max().unwrap_or(0);
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait_inbox(caller, after, Duration::from_secs(30)));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    let message_id = id();
    fixture.operator(OrchestrationAction::MessageSend {
        message_id: message_id.clone(), to_run_id: root.clone(), kind: MessageKind::Instruction,
        text: "PRIVATE MESSAGE BODY".into(), in_reply_to: None,
        original_task_revision: None, original_claim_id: None,
    });
    let counts = match poll_wait_once(waiting.as_mut()).await {
        std::task::Poll::Ready(result) => result.unwrap(),
        std::task::Poll::Pending => panic!("Relevant durable message did not wake inbox"),
    };
    assert!(counts.pending);
    assert_eq!(counts.through_seq, after + 1);
    assert_eq!(counts.counts.len(), 1);
    assert_eq!(counts.counts[0].kind, MessageKind::Instruction);
    let envelope = serde_json::to_value(counts).unwrap();
    assert_eq!(envelope.as_object().unwrap().len(), 4);
    assert!(!envelope.to_string().contains("PRIVATE"));
    assert_eq!(fixture.state().messages.iter().find(|message| message.message_id == message_id).unwrap().stage, DeliveryStage::Stored);
    assert_eq!(tokio::time::Instant::now(), started);
}

// Polling once establishes a deterministic pending-read barrier without letting
// Tokio auto-advance the paused clock. The real wait registers its own wakers.
async fn poll_wait_once<F: Future>(
    mut future: std::pin::Pin<&mut F>,
) -> std::task::Poll<F::Output> {
    std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx))).await
}

fn completed_wait(
    result: std::task::Poll<Result<OrchestrationWaitResponse, InspectionError>>,
) -> OrchestrationWaitResponse {
    match result {
        std::task::Poll::Ready(response) => response.unwrap(),
        std::task::Poll::Pending => panic!("Expected the consumer wait to complete"),
    }
}

#[tokio::test(start_paused = true)]
async fn wait_already_changed_revision_or_tasks_token_returns_at_zero_timeout() {
    let fixture = Fixture::new();
    let before = wait_request(&fixture, 0);
    let root = fixture.root();
    let task = fixture.create_task(&root, "Already available");
    let current = wait_request(&fixture, 0);
    let requests = [
        OrchestrationWaitRequest {
            after_tasks_token: current.after_tasks_token.clone(),
            ..before
        },
        OrchestrationWaitRequest {
            root_id: None,
            after_revision: current.after_revision,
            after_tasks_token: "older-task-token".into(),
            timeout_ms: 0,
        },
    ];
    let started = tokio::time::Instant::now();
    for request in requests {
        let mut waiting = Box::pin(fixture.service.wait(&request));
        let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
        assert!(response.changed);
        assert_eq!(response.revision, current.after_revision);
        assert_eq!(response.tasks_token, current.after_tasks_token);
    }
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(fixture.task(&root, &task.task_id).title, "Already available");
}

#[tokio::test(start_paused = true)]
async fn wait_zero_timeout_without_change_returns_current_cursor() {
    let fixture = Fixture::new();
    fixture.root();
    let request = wait_request(&fixture, 0);
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(!response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_eq!(response.tasks_token, request.after_tasks_token);
    assert_eq!(tokio::time::Instant::now(), started);
}

#[tokio::test(start_paused = true)]
async fn wait_watched_task_change_wakes_without_advancing_time() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let request = wait_request(&fixture, 3_000);
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());

    let task = fixture.create_task(&root, "Delivered by the watched service");
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(response.changed);
    assert_eq!(response.revision, fixture.state().revision);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(
        fixture.task(&root, &task.task_id).title,
        "Delivered by the watched service"
    );
}

#[tokio::test]
async fn wait_external_service_revision_is_visible_within_wait_budget() {
    let fixture = Fixture::new();
    let external = OrchestrationService::open(&fixture.configuration).unwrap();
    let request = wait_request(&fixture, 3_000);
    // A fresh store has no state file until its first committed mutation.
    let before = std::fs::read(fixture.service.base().join("state.json")).unwrap_or_default();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());

    let (new_root, attempt) = run_result(
        external
            .mutate_resolved(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationMutationRequest {
                    session_id: SESSION.into(),
                    expected_revision: Some(request.after_revision),
                    action: OrchestrationAction::SupervisorStart {
                        operation_id: id(),
                        selected_root_id: None,
                        target: Some(target()),
                        label: Some("External service supervisor".into()),
                    },
                },
                &ResolvedMutationInput { project: Some(fixture.project()), ..ResolvedMutationInput::default() },
            )
            .unwrap(),
    );
    let committed = fixture.state_bytes();
    assert_ne!(committed, before, "external machine mutation must persist a revision");
    // The writer is a different service/process notification domain. Only the
    // directory watcher can deliver this edge before the caller's deadline.
    let response = tokio::time::timeout(Duration::from_secs(1), waiting).await.unwrap().unwrap();
    assert!(response.changed);
    assert_eq!(response.revision, fixture.state().revision);
    assert!(response.revision > request.after_revision);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
    let delivered = fixture.run(&new_root);
    assert_eq!(delivered.run_id, new_root);
    assert_eq!(delivered.attempt, attempt);
    assert_eq!(delivered.kind, RunKind::Supervisor);
    assert_eq!(delivered.root_id, new_root);
    assert!(delivered.parent_run_id.is_none());
    assert_eq!(delivered.label, "External service supervisor");
    assert_eq!(fixture.state_bytes(), committed, "wait must not write machine state");
}

#[tokio::test]
async fn wait_external_markdown_replacement_delivers_task_without_revision_change() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let task = fixture.create_task(&root, "Original task title");
    let request = root_wait_request(&fixture, &root, 3_000);
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());

    let path = fixture.service.base().join("tasks").join(format!("{root}.md"));
    let replacement = path.with_extension("replacement");
    let document = std::fs::read_to_string(&path).unwrap();
    let updated = document.replace("Original task title", "Externally revised task title");
    assert_ne!(updated, document);
    std::fs::write(&replacement, updated).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    assert_eq!(fixture.state().revision, request.after_revision);
    let response = tokio::time::timeout(Duration::from_secs(1), waiting).await.unwrap().unwrap();
    assert!(response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_ne!(response.tasks_token, request.after_tasks_token);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token_for_root(&root).unwrap());
    assert_eq!(
        fixture.task(&root, &task.task_id).title,
        "Externally revised task title"
    );
    assert_eq!(fixture.task(&root, &task.task_id).body, task.body);
}

#[tokio::test(start_paused = true)]
async fn wait_short_timeout_returns_unchanged_at_the_requested_deadline() {
    let fixture = Fixture::new();
    fixture.root();
    let request = wait_request(&fixture, 20);
    let started = tokio::time::Instant::now();
    let before = fixture.state_bytes();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(19)).await;
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(1)).await;
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(!response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_eq!(response.tasks_token, request.after_tasks_token);
    assert_eq!(tokio::time::Instant::now() - started, Duration::from_millis(20));
    assert_eq!(fixture.state_bytes(), before);
}

#[tokio::test(start_paused = true)]
async fn wait_oversized_timeout_is_clipped_to_the_protocol_maximum() {
    let fixture = Fixture::new();
    let request = wait_request(&fixture, u32::MAX);
    let started = tokio::time::Instant::now();
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(29_999)).await;
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    tokio::time::advance(Duration::from_millis(1)).await;
    let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
    assert!(!response.changed);
    assert_eq!(response.revision, request.after_revision);
    assert_eq!(response.tasks_token, request.after_tasks_token);
    assert_eq!(tokio::time::Instant::now() - started, Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn wait_cancellation_preserves_state_and_next_wait_rediscovers_change() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let original = fixture.create_task(&root, "Existing task");
    let request = wait_request(&fixture, 3_000);
    let started = tokio::time::Instant::now();
    let before = fixture.state_bytes();
    let before_task = fixture.task(&root, &original.task_id);
    let mut waiting = Box::pin(fixture.service.wait(&request));
    assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
    drop(waiting);
    assert_eq!(fixture.state_bytes(), before);
    assert_eq!(
        fixture.task(&root, &original.task_id).task_revision,
        before_task.task_revision
    );

    let arrived = fixture.create_task(&root, "Arrived after cancellation");
    let mut next_wait = Box::pin(fixture.service.wait(&request));
    let response = completed_wait(poll_wait_once(next_wait.as_mut()).await);
    assert!(response.changed);
    assert_eq!(response.revision, fixture.state().revision);
    assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(
        fixture.task(&root, &arrived.task_id).title,
        "Arrived after cancellation"
    );
}

#[tokio::test(start_paused = true)]
async fn wait_subscription_edges_do_not_lose_back_to_back_changes() {
    for mutate_before_first_poll in [true, false] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let request = wait_request(&fixture, 3_000);
        let started = tokio::time::Instant::now();
        let mut waiting = Box::pin(fixture.service.wait(&request));
        if !mutate_before_first_poll {
            assert!(poll_wait_once(waiting.as_mut()).await.is_pending());
        }
        // Cover both a commit before subscription and retained notifications
        // after the first unchanged read, before the consumer is polled again.
        let first = fixture.create_task(&root, "First edge arrival");
        let second = fixture.create_task(&root, "Second edge arrival");
        let response = completed_wait(poll_wait_once(waiting.as_mut()).await);
        assert!(response.changed);
        assert_eq!(response.revision, fixture.state().revision);
        assert_eq!(response.tasks_token, fixture.service.store.tasks_token().unwrap());
        assert_eq!(tokio::time::Instant::now(), started);
        assert_eq!(fixture.task(&root, &first.task_id).title, "First edge arrival");
        assert_eq!(fixture.task(&root, &second.task_id).title, "Second edge arrival");
    }
}

#[test]
fn recovery_without_pending_intents_does_not_notify_subscribers() {
    let fixture = Fixture::new();
    fixture.root();
    let receiver = fixture.service.subscribe();
    let before = fixture.state_bytes();
    for _ in 0..3 {
        fixture.service.recover_intents().unwrap();
        assert!(!receiver.has_changed().unwrap());
        assert_eq!(fixture.state_bytes(), before);
    }
}

#[test]
fn recovery_observes_external_revision_once() {
    let fixture = Fixture::new();
    let external = OrchestrationService::open(&fixture.configuration).unwrap();
    let mut receiver = external.subscribe();
    let initial_revision = *receiver.borrow();
    fixture.root();
    let revision = fixture.state().revision;
    assert!(revision > initial_revision);
    assert!(!receiver.has_changed().unwrap());
    let before = fixture.state_bytes();

    external.recover_intents().unwrap();
    assert!(receiver.has_changed().unwrap());
    assert_eq!(*receiver.borrow_and_update(), revision);
    assert_eq!(fixture.state_bytes(), before);
    external.recover_intents().unwrap();
    assert!(!receiver.has_changed().unwrap());
    assert_eq!(fixture.state_bytes(), before);
}

#[test]
fn recovery_publishes_assignment_changes_before_later_recovery_error() {
    let fixture = Fixture::new();
    let roots = [fixture.root(), fixture.root()];
    let task_ids = [id(), id()];
    let paths = roots
        .each_ref()
        .map(|root| fixture.service.base().join("tasks").join(format!("{root}.md")));
    for index in 0..2 {
        // A directory in place of Markdown deterministically interrupts the
        // public assignment after its pending journal has been saved.
        std::fs::create_dir(&paths[index]).unwrap();
        assert!(
            fixture
                .apply(
                    &Actor::Operator(OperatorOrigin::Browser),
                    OrchestrationAction::TaskAssign {
                        root_id: roots[index].clone(),
                        task_id: task_ids[index].clone(),
                        title: format!("Interrupted assignment {index}"),
                        description: "Durable recovery notification".into(),
                    },
                )
                .is_err()
        );
    }
    assert_eq!(fixture.state().assignment_intents.len(), 2);
    std::fs::remove_dir(&paths[0]).unwrap();
    let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
    let mut receiver = reopened.subscribe();
    let initial_revision = *receiver.borrow();

    assert!(reopened.recover_intents().is_err());
    let state = fixture.state();
    assert!(state.revision > initial_revision);
    assert!(receiver.has_changed().unwrap());
    assert_eq!(*receiver.borrow_and_update(), state.revision);
    assert_eq!(state.assignment_intents.len(), 1);
    assert_eq!(state.assignment_intents[0].task_id, task_ids[1]);
    assert_eq!(fixture.task(&roots[0], &task_ids[0]).title, "Interrupted assignment 0");
    assert!(state.messages.iter().any(|message| {
        message.message_id == format!("assign-{}", task_ids[0])
    }));

    // The remaining I/O error alone is not a change and must not self-notify.
    let before = fixture.state_bytes();
    assert!(reopened.recover_intents().is_err());
    assert!(!receiver.has_changed().unwrap());
    assert_eq!(fixture.state_bytes(), before);

    std::fs::remove_dir(&paths[1]).unwrap();
    reopened.recover_intents().unwrap();
    let state = fixture.state();
    assert!(receiver.has_changed().unwrap());
    assert_eq!(*receiver.borrow_and_update(), state.revision);
    assert!(state.assignment_intents.is_empty());
    assert_eq!(fixture.task(&roots[1], &task_ids[1]).title, "Interrupted assignment 1");
    reopened.recover_intents().unwrap();
    assert!(!receiver.has_changed().unwrap());
}

#[test]
fn recovery_conflicts_on_changed_task_even_when_externally_checked() {
    for already_checked in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (task, run_id, actor) = fixture.working(&root);
        fixture.apply(&actor, fixture.successful_result(&actor))
            .unwrap();
        let intent_id = fixture.pending_intent(&root, &task, &run_id);
        // A canonical external edit may race a durable acceptance intent; the
        // content API correctly refuses that intent rather than providing a bypass.
        let edited = {
            let locked = fixture.service.store.lock().unwrap();
            locked.tasks(&root).unwrap().update(&task.task_id, &task.task_revision,
                Some("Changed while acceptance was interrupted"), None).unwrap()
        };
        assert_ne!(edited.task_revision, task.task_revision);
        if already_checked {
            let locked = fixture.service.store.lock().unwrap();
            locked
                .tasks(&root)
                .unwrap()
                .check(&task.task_id, &edited.task_revision, true)
                .unwrap();
        }
        let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
        let mut receiver = reopened.subscribe();
        reopened.recover_intents().unwrap();
        let state = reopened.store.lock().unwrap().read().unwrap();
        let run = &state.runs[run_index(&state, &run_id).unwrap()];
        assert_eq!(run.stage, RunStage::Reported);
        assert!(run.close_reason.is_none());
        assert_eq!(state.task_intents.len(), 1);
        assert_eq!(state.task_intents[0].intent_id, intent_id);
        assert_eq!(state.task_intents[0].state, IntentState::Conflict);
        assert_eq!(
            state.task_intents[0].expected_task_revision,
            task.task_revision
        );
        assert_eq!(fixture.task(&root, &task.task_id).checked, already_checked);
        assert!(receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), state.revision);
        let recovered_revision = state.revision;
        reopened.recover_intents().unwrap();
        assert_eq!(
            reopened.store.lock().unwrap().read().unwrap().revision,
            recovered_revision
        );
        assert!(!receiver.has_changed().unwrap());
    }
}

#[test]
fn recovery_applies_unchanged_pending_acceptance_once() {
    for check_phase in 0..3 {
        let fixture = Fixture::new();
        let root = fixture.root();
        let (task, run_id, actor) = fixture.working(&root);
        // Pending from unchecked Markdown, crash after writing the exact check,
        // or acceptance explicitly requested on the already checked item.
        if check_phase == 2 {
            fixture
                .service
                .store
                .lock()
                .unwrap()
                .tasks(&root)
                .unwrap()
                .check(&task.task_id, &task.task_revision, true)
                .unwrap();
            fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
            fixture.pending_intent(&root, &fixture.task(&root, &task.task_id), &run_id);
        } else {
            fixture.apply(&actor, fixture.successful_result(&actor)).unwrap();
            fixture.pending_intent(&root, &task, &run_id);
            if check_phase == 1 {
                fixture
                    .service
                    .store
                    .lock()
                    .unwrap()
                    .tasks(&root)
                    .unwrap()
                    .check(&task.task_id, &task.task_revision, true)
                    .unwrap();
            }
        }
        let reopened = OrchestrationService::open(&fixture.configuration).unwrap();
        let mut receiver = reopened.subscribe();
        reopened.recover_intents().unwrap();
        assert!(fixture.task(&root, &task.task_id).checked);
        assert_eq!(
            fixture.run(&run_id).close_reason,
            Some(CloseReason::Accepted)
        );
        assert!(fixture.state().task_intents.is_empty());
        let revision = fixture.state().revision;
        assert!(receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), revision);
        reopened.recover_intents().unwrap();
        assert_eq!(fixture.state().revision, revision);
        assert!(!receiver.has_changed().unwrap());
    }
}
