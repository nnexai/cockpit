use super::*;

const NOW: &str = "2026-11-08T12:00:00Z";

fn task(id: &str) -> TaskView {
    TaskView {
        task: Task {
            task_id: id.into(), title: format!("Task {id}"), body: String::new(),
            description: String::new(), description_editable: true, description_diagnostic: None,
            steps: vec![], step_progress: None, steps_diagnostic: None, depends_on: vec![],
            follow_up_of: None, relations_diagnostic: None, checked: false, line: 1,
            task_revision: "rev".into(), diagnostic: None,
        },
        project: None, lane: TaskLane::Ready, current_run_id: None,
        dependencies: TaskDependencies { state: TaskDependencyState::None, unmet: vec![], problems: vec![] },
        scheduled: false, attempt: None, waiting: None, worker_waiting: None,
    }
}

fn worker(stage: RunStage) -> Run {
    Run {
        session_id: "session".into(), brief: String::new(), run_id: "worker".into(),
        kind: RunKind::Worker, label: "worker".into(), root_id: "root".into(),
        parent_run_id: Some("root".into()), task_id: Some("task".into()), attempt: 1,
        assignment_generation: Some(1), launch_config: Default::default(), task_revision_at_start: None,
        stage, close_reason: None, dispatch: None, target: None, setup: None, cleanup_policy: None,
        last_report: None, result: None, source_output: None, integration: Integration::Pending,
        annotations: vec![], project: None, checkout: None, attachment: None, checkout_claim_id: None,
        resume: None, retirement: None, created_at: NOW.into(), updated_at: NOW.into(),
    }
}

fn snapshot() -> OrchestrationSnapshot {
    OrchestrationSnapshot {
        session_id: "session".into(), state_store_id: "store".into(), revision: 1,
        tasks_token: "token".into(), roots: vec![],
        board: Some(TaskBoard { root_id: "root".into(), path: String::new(), doc_revision: String::new(),
            unidentified_items: 0, diagnostics: vec![], tasks: vec![task("task")] }),
        runs: vec![], messages: vec![], questions: vec![], unreflected_inputs: vec![],
        task_ownership: vec![], directives: vec![], thread_links: vec![], intents: vec![],
        assignment_intents: vec![], cleanup: vec![],
        restoration: RestorationSummary { reattached: 0, total_attachments: 0, cleanup_pending: 0 },
        runtime: RuntimeObservation::Fresh { endpoint_identity: "endpoint".into(), observed_at: NOW.into(), runs: vec![] },
        unmanaged_agents: vec![], attention: vec![],
    }
}

fn runtime() -> RuntimeView {
    RuntimeView { endpoint_identity: "endpoint".into(), boot_id: None, peer_process: None,
        focused_workspace_id: None, workspaces: vec![], panes: vec![] }
}

fn observe(s: &mut OrchestrationSnapshot, presence: Presence, status: Option<&str>) {
    s.runtime = RuntimeObservation::Fresh { endpoint_identity: "endpoint".into(), observed_at: NOW.into(),
        runs: vec![RunObservation { run_id: "worker".into(), presence, actual_agent: true,
            workspace_id: None, workspace_label: None, tab_id: None, tab_label: None, pane_id: None,
            agent_status: status.map(str::to_owned), state_changed_at: Some(NOW.into()) }] };
}

fn active(s: &mut OrchestrationSnapshot, stage: RunStage) {
    s.board.as_mut().unwrap().tasks[0].current_run_id = Some("worker".into());
    s.runs = vec![worker(stage)];
}

fn question(run: &str, id: &str, thread: Option<&str>) -> QuestionStatus {
    QuestionStatus { run_id: run.into(), question_message_id: id.into(), asked_at: NOW.into(),
        thread_id: thread.map(str::to_owned), receipt: QuestionReceipt::Unresolved }
}

fn directive(id: &str, scope: DirectiveScope) -> Directive {
    Directive { directive_id: id.into(), root_id: "root".into(), scope, text: "Follow guidance".into(),
        source: DirectiveSource::Operator, thread_id: None, message_id: None,
        created_by: ActorRef::Operator, created_at: NOW.into(), removed_at: None }
}

fn digest_at(s: &OrchestrationSnapshot, rt: Option<&RuntimeView>, now: &str) -> BoardDigest {
    digest(s, "root", rt, now).unwrap()
}

#[test]
fn maps_task_and_runtime_states() {
    let mut s = snapshot();
    let rt = runtime();
    assert_eq!(digest_at(&s, None, NOW).tasks[0].worker, WorkerState::Unscheduled);
    s.task_ownership.push(TaskOwnership { root_id: "root".into(), task_id: "task".into(), generation: 0,
        owner: None, reopened_after_attempt: None, schedule: Some(TaskSchedule {
            operation_id: "op".into(), principal_key: "principal".into(), target: None,
            brief: String::new(), cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe,
            label: None, scheduled_at: NOW.into(), hold: Some(ScheduleHold { code: "capacity".into(),
                at: NOW.into(), dependency_task_id: None }),
        }) });
    let queued = digest_at(&s, None, NOW);
    assert_eq!(queued.tasks[0].worker, WorkerState::Queued);
    assert!(queued.tasks[0].placement.as_deref().unwrap().contains("capacity"));
    active(&mut s, RunStage::Starting);
    assert_eq!(digest_at(&s, None, NOW).tasks[0].worker, WorkerState::Starting);
    s.runs[0].stage = RunStage::Working;
    assert_eq!(digest_at(&s, None, NOW).tasks[0].worker, WorkerState::Unknown);
    for (status, expected) in [("working", WorkerState::Working), ("idle", WorkerState::IdleNoReport),
        ("done", WorkerState::IdleNoReport), ("blocked", WorkerState::Blocked)] {
        observe(&mut s, Presence::Present, Some(status));
        assert_eq!(digest_at(&s, Some(&rt), NOW).tasks[0].worker, expected, "{status}");
    }
    observe(&mut s, Presence::Missing, None);
    assert_eq!(digest_at(&s, Some(&rt), NOW).tasks[0].worker, WorkerState::Gone);
    s.runtime = RuntimeObservation::Fresh { endpoint_identity: "endpoint".into(), observed_at: NOW.into(), runs: vec![] };
    assert_eq!(digest_at(&s, Some(&rt), NOW).tasks[0].worker, WorkerState::Gone);
    s.questions.push(question("worker", "q1", None));
    assert_eq!(digest_at(&s, Some(&rt), NOW).tasks[0].worker, WorkerState::QuestionPending);
    s.questions.clear();
    s.runs[0].stage = RunStage::Reported;
    assert_eq!(digest_at(&s, Some(&rt), NOW).tasks[0].worker, WorkerState::AwaitingReview);
    s.thread_links.push(ThreadLink { root_id: "root".into(), task_id: "task".into(), thread_id: "thread".into() });
    s.questions.push(question("root", "q2", Some("thread")));
    let waiting = digest_at(&s, Some(&rt), NOW);
    assert_eq!(waiting.tasks[0].worker, WorkerState::WaitingOnUser);
    assert!(!waiting.tasks[0].counts_as_open);
    assert_eq!(waiting.open_work, 0);
    s.board.as_mut().unwrap().tasks[0].lane = TaskLane::Accepted;
    assert!(digest_at(&s, Some(&rt), NOW).tasks.is_empty());
}

#[test]
fn worker_wait_claim_is_independent_of_runtime_and_questions() {
    let mut s = snapshot();
    let rt = runtime();
    active(&mut s, RunStage::Working);
    s.board.as_mut().unwrap().tasks[0].worker_waiting = Some(TaskWait {
        cause: TaskWaitCause::WorkerWaiting, since: Some(NOW.into()), held_by: vec![],
        code: None, detail: Some("Waiting for external review".into()), next_automatic: false,
    });
    for (status, expected) in [("idle", WorkerState::Waiting), ("done", WorkerState::Waiting),
        ("working", WorkerState::Working), ("blocked", WorkerState::Blocked)] {
        observe(&mut s, Presence::Present, Some(status));
        let digest = digest_at(&s, Some(&rt), NOW);
        let row = &digest.tasks[0];
        assert_eq!(row.worker, expected, "{status}");
        assert_eq!(row.observed_status.as_deref(), Some(status));
        assert_eq!(row.worker_waiting.as_ref().unwrap().detail.as_deref(), Some("Waiting for external review"));
        assert!(row.counts_as_open);
        assert_eq!(digest.open_work, 1);
        assert!(digest.text.contains(&format!("observed status: {status}")));
        assert!(digest.text.contains("worker-stated wait: Waiting for external review"));
    }
    observe(&mut s, Presence::Missing, Some("idle"));
    assert_eq!(digest_at(&s, Some(&rt), NOW).tasks[0].worker, WorkerState::Gone);
    observe(&mut s, Presence::Present, Some("working"));
    s.questions.push(question("worker", "q1", None));
    let pending = digest_at(&s, Some(&rt), NOW);
    assert_eq!(pending.tasks[0].worker, WorkerState::QuestionPending);
    assert_eq!(pending.tasks[0].observed_status.as_deref(), Some("working"));
    assert!(pending.text.contains("worker-stated wait: Waiting for external review"));
    assert_eq!(pending.open_work, 1);
    s.thread_links.push(ThreadLink { root_id: "root".into(), task_id: "task".into(), thread_id: "thread".into() });
    s.questions.push(question("root", "q2", Some("thread")));
    let user_question = digest_at(&s, Some(&rt), NOW);
    assert_eq!(user_question.tasks[0].worker, WorkerState::WaitingOnUser);
    assert_eq!(user_question.open_work, 0);
    assert!(user_question.text.contains("worker-stated wait: Waiting for external review"));
}

#[test]
fn wait_claim_and_resume_change_digest_key_even_when_question_has_priority() {
    let mut s = snapshot();
    active(&mut s, RunStage::Working);
    s.questions.push(question("worker", "q1", None));
    let before = digest_at(&s, None, NOW);
    s.board.as_mut().unwrap().tasks[0].worker_waiting = Some(TaskWait {
        cause: TaskWaitCause::WorkerWaiting, since: Some(NOW.into()), held_by: vec![],
        code: None, detail: Some("First claim".into()), next_automatic: false,
    });
    let waiting = digest_at(&s, None, NOW);
    assert_eq!(waiting.tasks[0].worker, WorkerState::QuestionPending);
    assert_ne!(before.key, waiting.key);
    s.board.as_mut().unwrap().tasks[0].worker_waiting.as_mut().unwrap().detail = Some("Changed claim".into());
    let changed = digest_at(&s, None, NOW);
    assert_ne!(waiting.key, changed.key);
    s.board.as_mut().unwrap().tasks[0].worker_waiting = None;
    assert_eq!(before.key, digest_at(&s, None, NOW).key);
}

#[test]
fn key_ignores_time_and_revision_but_tracks_board_changes() {
    let mut s = snapshot();
    active(&mut s, RunStage::Working);
    let first = digest_at(&s, None, NOW);
    s.revision += 1;
    s.runs[0].updated_at = "2026-11-08T12:05:00Z".into();
    let later = digest_at(&s, None, "2026-11-08T12:20:00Z");
    assert_eq!(first.key, later.key);
    assert_eq!(first.key.len(), 16);
    assert!(first.key.chars().all(|c| c.is_ascii_hexdigit()));
    s.runs[0].stage = RunStage::Reported;
    let stage = digest_at(&s, None, NOW);
    assert_ne!(first.key, stage.key);
    s.questions.push(question("worker", "question-1", None));
    let with_question = digest_at(&s, None, NOW);
    assert_ne!(stage.key, with_question.key);
    s.questions[0].question_message_id = "question-2".into();
    let changed_question = digest_at(&s, None, NOW);
    assert_ne!(with_question.key, changed_question.key);
    s.directives.push(directive("directive-1", DirectiveScope::Task { task_id: "task".into() }));
    let with_directive = digest_at(&s, None, NOW);
    assert_ne!(changed_question.key, with_directive.key);
    assert_eq!(with_directive.tasks[0].directive_ids, vec!["directive-1"]);
    let directive_text = &s.directives.last().unwrap().text;
    assert!(with_directive.text.contains(&format!("user directive [directiv…]: {directive_text}"))
        || with_directive.text.contains(&format!(": {directive_text}")), "{}", with_directive.text);
    s.directives[0].removed_at = Some(NOW.into());
    assert_ne!(with_directive.key, digest_at(&s, None, NOW).key);
}

#[test]
fn digest_text_is_bounded_and_unicode_safe() {
    let mut s = snapshot();
    let board = s.board.as_mut().unwrap();
    board.tasks = (0..40).map(|i| { let mut t = task(&format!("{i:02}")); t.task.title = "界".repeat(200); t }).collect();
    let d = digest_at(&s, None, NOW);
    assert_eq!(d.open_work, 40);
    assert_eq!(d.tasks.len(), 40);
    assert!(d.text.len() <= 4096);
    assert!(d.text.lines().filter(|line| line.starts_with("- ")).count() <= 25);
    assert!(d.omitted_tasks > 0);
    for line in d.text.lines().filter(|line| line.starts_with("- ")) {
        let title = line.split('"').nth(1).unwrap();
        assert!(title.chars().count() <= 80);
    }
}

#[test]
fn bounded_text_is_unicode_safe() {
    assert_eq!(shorten("áéíó", 3), "áé…");
    assert_eq!(shorten("a\nb", 3), "a b");
    let mut text = "x".repeat(4094);
    assert!(add_line(&mut text, "y"));
    assert_eq!(text.len(), 4096);
    assert!(!add_line(&mut text, "z"));
}

#[test]
fn attention_order_and_elapsed_age() {
    assert!(priority(WorkerState::AwaitingReview) < priority(WorkerState::Working));
    assert!(priority(WorkerState::Working) < priority(WorkerState::WaitingOnUser));
    assert_eq!(age("2026-11-08T12:18:00Z", Some(NOW)), " 18m");
    assert_eq!(age("invalid", Some(NOW)), "");
}

#[test]
fn retained_resources_alone_are_not_open_work_and_are_summarized() {
    let mut s = snapshot();
    s.board.as_mut().unwrap().tasks[0].lane = TaskLane::Accepted;
    s.runs.push(worker(RunStage::Closed));
    let items = (0..7).map(|i| CleanupResourceSummary {
        resource_id: format!("resource-{i}"), kind: OwnedResourceKind::Worktree, ownership: ResourceOwnership::Owned,
        cleanup: CleanupItem { operation_id: None, state: CleanupState::Kept, effects: vec![], updated_at: NOW.into(),
            reason: Some(CleanupReason { class: CleanupReasonClass::UserWork, code: "kept_reason".into(), message: String::new() }) },
    }).collect();
    s.cleanup.push(RunCleanupSummary { run_id: "worker".into(), items, removed_count: 0, waiting_count: 0, kept_count: 7, unknown_count: 0 });
    let d = digest_at(&s, None, NOW);
    assert_eq!(d.open_work, 0);
    assert_eq!(d.retained.len(), 7);
    assert_eq!(d.text.lines().filter(|line| line.starts_with("Retained: worktree")).count(), 5);
    assert!(d.text.contains("Retained: 2 more"), "{}", d.text);
}
