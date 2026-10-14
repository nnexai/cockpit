use super::*;
use sha2::{Digest, Sha256};
use super::super::credentials::{JobAuth, SecretBytes, test_job_auth, test_job_credential};

fn fixture_uuid(scope: &str, id: &str, subject: Option<&str>) -> String {
    let digest = Sha256::digest(format!("{scope}:{id}:{}", subject.unwrap_or("")));
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn fixture_attachment_id(id: &str) -> String {
    fixture_uuid("attachment", id, None)
}

fn fixture_auth(id: &str) -> JobAuth {
    let parent = (id != "root").then_some("root");
    let run = run(id, parent, RunStage::Working);
    let role = if parent.is_some() { JobRole::Worker } else { JobRole::Root };
    test_job_credential(&run, role).1
}

fn conversation(id: &str) -> ConversationIdentity {
    ConversationIdentity { harness: "test-harness".into(), session_id: format!("conversation-{id}") }
}

fn native_process() -> NativeProcessIdentity {
    NativeProcessIdentity {
        pid: 41,
        start_ticks: 73,
        kernel_boot_id: Some("test-boot".into()),
    }
}

fn run(id: &str, parent: Option<&str>, stage: RunStage) -> Run {
    Run {
        session_id: "herdr-session".into(),
        brief: String::new(),
        run_id: id.into(),
        kind: if parent.is_some() {
            RunKind::Worker
        } else {
            RunKind::Supervisor
        },
        label: id.into(),
        root_id: "root".into(),
        parent_run_id: parent.map(str::to_owned),
        task_id: parent.map(|_| format!("task-{id}")),
        attempt: 1,
        task_revision_at_start: None,
        assignment_generation: parent.map(|_| 1),
        launch_config: cockpit_protocol::projects::OrchestrationConfiguration {
            harness_kind: "test-harness".into(), ..Default::default()
        },
        stage,
        close_reason: None,
        dispatch: None,
        target: None,
        setup: None,
        cleanup_policy: None,
        last_report: None,
        result: None,
        source_output: None,
        integration: Integration::Pending,
        annotations: Vec::new(),
        project: None,
        checkout: None,
        checkout_claim_id: None,
        resume: None,
        attachment: Some(ExecutionAttachment {
            attachment_id: fixture_attachment_id(id),
            epoch: 1,
            state: AttachmentState::Connected,
            launch_mode: None,
            location: None,
            conversation: Some(conversation(id)),
            process: Some(native_process()),
            process_scope: None,
            shell: None,
            machine_id: Some("test-machine".into()),
            launch_attempt: 1,
            updated_at: now(),
        }),
        retirement: None,
        created_at: now(),
        updated_at: now(),
    }
}

fn state() -> OrchestrationState {
    let mut state = OrchestrationState {
        revision: 0,
        runs: vec![
            run("root", None, RunStage::Active),
            run("worker", Some("root"), RunStage::Working),
            run("grandchild", Some("worker"), RunStage::Working),
            run("sibling", Some("root"), RunStage::Working),
        ],
        messages: Vec::new(),
        task_intents: Vec::new(),
        assignment_intents: Vec::new(),
        ..OrchestrationState::default()
    };
    for index in 0..state.runs.len() {
        let role = if index == 0 { JobRole::Root } else { JobRole::Worker };
        test_job_auth(&mut state, index, role);
    }
    state
}

fn main_actor(run_id: &str) -> Actor {
    Actor::Agent(AgentCaller {
        auth: fixture_auth(run_id),
        conversation: Some(conversation(run_id)),
        assignment_generation: (run_id != "root").then_some(1),
        endpoint_identity: "endpoint".into(),
        session_id: "herdr-session".into(),
        workspace_id: "space".into(),
        tab_id: "tab".into(),
        pane_id: "pane".into(),
        boot_id: None,
        terminal_id: None,
        native_session_id: None,
        actual_agent_kind: Some("omp".into()),
        process: Some(native_process()),
    })
}


fn report(id: &str, kind: ReportKind, destination: Option<&str>) -> OrchestrationAction {
    OrchestrationAction::Report {
        message_id: id.into(),
        kind,
        outcome: (kind == ReportKind::Result).then_some(ReportOutcome::Succeeded),
        summary: format!("summary-{id}"),
        plan: (kind == ReportKind::Initialized).then(|| "Inspect, change, then verify.".into()),
        source_output: (kind == ReportKind::Result).then(|| SourceOutput::NonCode {
            result_message_id: id.into(), task_revision: "task-revision".into(),
            artifacts: Vec::new(), focused_checks: Vec::new(),
        }),
        to_run_id: destination.map(str::to_owned),
        original_task_revision: None,
        original_claim_id: None,
    }
}

fn send(id: &str, kind: MessageKind, question: Option<&str>) -> OrchestrationAction {
    OrchestrationAction::MessageSend {
        message_id: id.into(),
        to_run_id: "worker".into(),
        kind,
        text: "Proceed with the approved choice".into(),
        in_reply_to: question.map(str::to_owned),
        original_task_revision: None,
        original_claim_id: None,
    }
}

fn assert_rejected_without_mutation(
    state: &mut OrchestrationState,
    action: &OrchestrationAction,
    code: &str,
) {
    let before = serde_json::to_value(&*state).unwrap();
    let operator = Actor::Operator(OperatorOrigin::Browser);
    assert_eq!(
        apply(state, &operator, None, false, action).unwrap_err().code,
        code
    );
    assert_eq!(serde_json::to_value(&*state).unwrap(), before);
}

fn mutation(
    state: &mut OrchestrationState,
    actor: &Actor,
    index: usize,
    action: &OrchestrationAction,
) -> OrchestrationActionResult {
    apply(state, actor, Some(index), false, action)
        .unwrap()
        .unwrap()
}

fn assert_error(
    state: &mut OrchestrationState,
    actor: &Actor,
    index: usize,
    action: &OrchestrationAction,
    code: &str,
) {
    assert_eq!(
        apply(state, actor, Some(index), false, action)
            .unwrap_err()
            .code,
        code
    );
}


fn root_answer(id: &str, question: Option<&str>) -> OrchestrationAction {
    OrchestrationAction::MessageSend {
        message_id: id.into(), to_run_id: "root".into(), kind: MessageKind::Answer,
        text: "The human chose the scoped safe option.".into(),
        in_reply_to: question.map(str::to_owned),
        original_task_revision: None, original_claim_id: None,
    }
}

#[test]
fn root_relays_exact_human_answer_and_capture_receipts_do_not_rewrite_reports() {
    let mut state = state();
    let root = main_actor("root");
    mutation(&mut state, &root, 0, &report("human-question", ReportKind::NeedsInput, None));
    mutation(&mut state, &root, 0, &report("later-progress", ReportKind::Progress, None));
    let answer = root_answer("human-answer", Some("human-question"));
    let result = mutation(&mut state, &root, 0, &answer);
    assert!(matches!(result, OrchestrationActionResult::Message { duplicate: false, .. }));
    let delivered = state.messages.last().unwrap();
    assert_eq!(delivered.to_run_id, "root");
    assert!(matches!(&delivered.from, ActorRef::Run { run_id } if run_id == "root"));
    assert_eq!(delivered.in_reply_to.as_deref(), Some("human-question"));
    assert_eq!(delivered.text, "The human chose the scoped safe option.");
    assert_eq!(state.runs[0].last_report.as_ref().unwrap().message_id, "later-progress");
    assert_eq!(state.runs[0].stage, RunStage::Active);
    let inbox = state.messages.iter().filter(|message| message.to_run_id == "root").collect::<Vec<_>>();
    let question = super::super::projection::question_status(&state.runs[0], &inbox, &state.messages).unwrap();
    assert!(matches!(question.receipt, QuestionReceipt::AnswerDelivered { .. }));
    let OrchestrationActionResult::Inbox { read_through_seq, .. } =
        mutation(&mut state, &root, 0, &OrchestrationAction::InboxPull { after_seq: 0, limit: 100 }) else { unreachable!() };
    mutation(&mut state, &root, 0, &OrchestrationAction::InboxAck { through_seq: read_through_seq });
    let inbox = state.messages.iter().filter(|message| message.to_run_id == "root").collect::<Vec<_>>();
    let question = super::super::projection::question_status(&state.runs[0], &inbox, &state.messages).unwrap();
    assert!(matches!(question.receipt, QuestionReceipt::AnswerAcknowledged { .. }));
    assert_eq!(state.runs[0].last_report.as_ref().unwrap().message_id, "later-progress");
}

#[test]
fn root_self_answer_requires_exact_unresolved_question_and_preserves_committed_replay() {
    let mut state = state();
    let root = main_actor("root");
    let answer = root_answer("answer", Some("question"));
    let before = serde_json::to_value(&state).unwrap();
    assert_error(&mut state, &root, 0, &answer, "question_not_current");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    mutation(&mut state, &root, 0, &report("question", ReportKind::Progress, None));
    assert_error(&mut state, &root, 0, &answer, "question_not_current");
    mutation(&mut state, &root, 0, &report("human-question", ReportKind::NeedsInput, None));
    assert_error(&mut state, &root, 0, &root_answer("missing-link", None), "invalid_message");
    assert_error(&mut state, &root, 0, &answer, "question_not_current");
    let answer = root_answer("answer", Some("human-question"));
    mutation(&mut state, &root, 0, &answer);
    let before = serde_json::to_value(&state).unwrap();
    assert_error(&mut state, &root, 0, &root_answer("second-answer", Some("human-question")), "question_already_answered");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    assert!(matches!(mutation(&mut state, &root, 0, &answer), OrchestrationActionResult::Message { duplicate: true, .. }));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    mutation(&mut state, &root, 0, &report("new-question", ReportKind::NeedsInput, None));
    assert_error(&mut state, &root, 0, &root_answer("late-answer", Some("human-question")), "question_not_current");
    assert!(matches!(mutation(&mut state, &root, 0, &answer), OrchestrationActionResult::Message { duplicate: true, .. }));
}

#[test]
fn worker_self_answer_and_other_sender_question_do_not_gain_root_answer_authority() {
    let mut state = state();
    let root = main_actor("root");
    let worker = main_actor("worker");
    mutation(&mut state, &worker, 1, &report("worker-question", ReportKind::NeedsInput, None));
    let before = serde_json::to_value(&state).unwrap();
    assert_error(&mut state, &worker, 1, &send("worker-self-answer", MessageKind::Answer, Some("worker-question")), "actor_forbidden");
    assert_error(&mut state, &root, 0, &root_answer("wrong-sender", Some("worker-question")), "question_not_current");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    mutation(&mut state, &root, 0, &report("root-question", ReportKind::NeedsInput, None));
    mutation(&mut state, &root, 0, &send("descendant-answer", MessageKind::Answer, Some("worker-question")));
    assert_eq!(state.messages.last().unwrap().to_run_id, "worker");
}

#[test]
fn root_batch_result_is_diagnostic_and_preserves_active_delegation() {
    for outcome in [ReportOutcome::Succeeded, ReportOutcome::Failed] {
        let mut state = state();
        let root = main_actor("root");
        let mut batch = report("batch-result", ReportKind::Result, None);
        if let OrchestrationAction::Report { outcome: result_outcome, .. } = &mut batch {
            *result_outcome = Some(outcome);
        }
        mutation(&mut state, &root, 0, &batch);
        assert_eq!(state.runs[0].stage, RunStage::Active);
        assert!(state.runs[0].result.is_none());
        assert!(state.runs[0].source_output.is_none());
        assert_eq!(state.runs[0].integration, Integration::Pending);
        assert_eq!(state.runs[0].last_report.as_ref().unwrap().outcome, Some(outcome));
        assert_eq!(state.messages.last().unwrap().to_run_id, "root");
        let before = serde_json::to_value(&state).unwrap();
        assert!(matches!(mutation(&mut state, &root, 0, &batch), OrchestrationActionResult::Message { duplicate: true, .. }));
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        mutation(&mut state, &root, 0, &send("next-batch-instruction", MessageKind::Instruction, None));
        assert_eq!(state.messages.last().unwrap().to_run_id, "worker");
        let worker = main_actor("worker");
        mutation(&mut state, &worker, 1, &report("worker-result", ReportKind::Result, None));
        assert_eq!(state.runs[1].stage, RunStage::Reported);
        assert!(state.runs[1].result.is_some());
        assert!(state.runs[1].source_output.is_some());
        assert_eq!(state.runs[0].stage, RunStage::Active);
    }
}

#[test]
fn answers_require_explicit_links_and_other_messages_forbid_links() {
    let mut state = state();
    mutation(
        &mut state,
        &main_actor("worker"),
        1,
        &report("question", ReportKind::NeedsInput, None),
    );
    for question in [None, Some(""), Some(" \t")] {
        assert_rejected_without_mutation(
            &mut state,
            &send("unlinked", MessageKind::Answer, question),
            "invalid_message",
        );
    }
    for kind in [MessageKind::Instruction, MessageKind::CancelRequest] {
        for question in [Some("question"), Some("")] {
            assert_rejected_without_mutation(
                &mut state,
                &send("not-answer", kind, question),
                "invalid_message",
            );
        }
    }
}

#[test]
fn answers_reject_old_questions_but_committed_retries_survive_new_questions() {
    let mut state = state();
    let worker = main_actor("worker");
    let operator = Actor::Operator(OperatorOrigin::Browser);
    mutation(&mut state, &worker, 1, &report("q1", ReportKind::NeedsInput, None));
    let answer = send("a1", MessageKind::Answer, Some("q1"));
    let first = apply(&mut state, &operator, None, false, &answer).unwrap().unwrap();
    assert!(matches!(first, OrchestrationActionResult::Message { duplicate: false, seq: 1, .. }));
    assert_eq!(state.messages.last().unwrap().in_reply_to.as_deref(), Some("q1"));
    mutation(&mut state, &worker, 1, &report("q2", ReportKind::NeedsInput, None));
    assert_rejected_without_mutation(
        &mut state,
        &send("late", MessageKind::Answer, Some("q1")),
        "question_not_current",
    );
    let before = serde_json::to_value(&state).unwrap();
    let retry = apply(&mut state, &operator, None, false, &answer).unwrap().unwrap();
    assert!(matches!(retry, OrchestrationActionResult::Message { duplicate: true, seq: 1, .. }));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    for question in [Some("q2"), None, Some("")] {
        assert_rejected_without_mutation(
            &mut state,
            &send("a1", MessageKind::Answer, question),
            "message_id_conflict",
        );
    }
    let mut changed_text = answer;
    if let OrchestrationAction::MessageSend { text, .. } = &mut changed_text {
        *text = "A different choice".into();
    }
    assert_rejected_without_mutation(&mut state, &changed_text, "message_id_conflict");
    apply(
        &mut state,
        &operator,
        None,
        false,
        &send("a2", MessageKind::Answer, Some("q2")),
    ).unwrap();
    assert_eq!(state.messages.last().unwrap().in_reply_to.as_deref(), Some("q2"));
    assert_eq!(state.runs[1].last_report.as_ref().unwrap().message_id, "q2");
}

#[test]
fn answers_require_an_open_current_main_needs_input_report() {
    let mut state = state();
    let worker = main_actor("worker");
    let answer = send("answer", MessageKind::Answer, Some("question"));
    assert_rejected_without_mutation(&mut state, &answer, "question_not_current");
    mutation(&mut state, &worker, 1, &report("question", ReportKind::Progress, None));
    assert_rejected_without_mutation(&mut state, &answer, "question_not_current");
    mutation(&mut state, &worker, 1, &report("question-ni", ReportKind::NeedsInput, None));
    state.runs[1].stage = RunStage::Closed;
    assert_rejected_without_mutation(
        &mut state,
        &send("answer", MessageKind::Answer, Some("question-ni")),
        "question_not_current",
    );
}

#[test]
fn stale_questions_cannot_replace_the_current_question() {
    let mut state = state();
    let actor = main_actor("worker");
    mutation(&mut state, &actor, 1, &report("current-question", ReportKind::NeedsInput, None));
    let before = serde_json::to_value(&state).unwrap();
    assert_eq!(apply(&mut state, &actor, Some(1), true,
        &report("stale-question", ReportKind::NeedsInput, None)).unwrap_err().code, "attempt_stale");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    assert_rejected_without_mutation(&mut state,
        &send("answer", MessageKind::Answer, Some("stale-question")), "question_not_current");
    apply(&mut state, &Actor::Operator(OperatorOrigin::Browser), None, false,
        &send("answer", MessageKind::Answer, Some("current-question"))).unwrap();
    assert_eq!(state.runs[1].last_report.as_ref().unwrap().message_id, "current-question");
}

#[test]
fn dedupe_rejects_changed_payload_and_preserves_original_report_after_stage_change() {
    let mut state = state();
    let actor = main_actor("worker");
    let ready = report("initialized", ReportKind::Initialized, None);
    mutation(&mut state, &actor, 1, &ready);
    state.runs[1].stage = RunStage::Working;
    let receipt = mutation(&mut state, &actor, 1, &ready);
    assert!(matches!(
        receipt,
        OrchestrationActionResult::Message {
            duplicate: true,
            seq: 1,
            ..
        }
    ));
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.runs[1].stage, RunStage::Working);
    let OrchestrationAction::Report {
        mut summary,
        message_id,
        kind,
        outcome,
        plan,
        source_output,
        to_run_id,
        original_task_revision,
        original_claim_id,
    } = ready
    else {
        unreachable!()
    };
    summary.push_str(" altered");
    assert_error(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::Report {
            message_id,
            kind,
            outcome,
            summary,
            plan,
            source_output,
            to_run_id,
            original_task_revision,
            original_claim_id,
        },
        "message_id_conflict",
    );
    assert_eq!(
        state.runs[1].last_report.as_ref().unwrap().summary,
        "summary-initialized"
    );
}

#[test]
fn append_keys_by_sender_and_sequences_by_recipient() {
    let mut state = state();
    let first = append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "same", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, escalated_from: None })
    .unwrap();
    assert!(matches!(
        first,
        OrchestrationActionResult::Message {
            seq: 1,
            duplicate: false,
            ..
        }
    ));
    let duplicate = append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "same", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, escalated_from: None })
    .unwrap();
    assert!(matches!(
        duplicate,
        OrchestrationActionResult::Message {
            seq: 1,
            duplicate: true,
            ..
        }
    ));
    append(&mut state, AppendMessage { from: ActorRef::Run {
        run_id: "root".into(),
    }, to_run_id: "worker", message_id: "same", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, escalated_from: None })
    .unwrap();
    append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "sibling", message_id: "other", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, escalated_from: None })
    .unwrap();
    assert_eq!(
        state
            .messages
            .iter()
            .map(|message| message.seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 1]
    );
    assert_eq!(
        append(&mut state, AppendMessage {
            from: ActorRef::Operator,
            to_run_id: "sibling",
            message_id: "same",
            kind: MessageKind::Instruction,
            text: "hello",
            in_reply_to: None,
            report: None,
            stale: false,
            escalated_from: None,
        })
        .unwrap_err()
        .code,
        "message_id_conflict"
    );
    assert_eq!(
        append(&mut state, AppendMessage {
            from: ActorRef::Operator,
            to_run_id: "worker",
            message_id: "same",
            kind: MessageKind::CancelRequest,
            text: "hello",
            in_reply_to: None,
            report: None,
            stale: false,
            escalated_from: None,
        })
        .unwrap_err()
        .code,
        "message_id_conflict"
    );
}

#[test]
fn append_bounds_utf8_bytes_and_rejects_sequence_overflow() {
    let mut state = state();
    let oversized = "é".repeat(MAX_TEXT / 2 + 1);
    assert_eq!(
        append(&mut state, AppendMessage {
            from: ActorRef::Operator,
            to_run_id: "worker",
            message_id: "large",
            kind: MessageKind::Instruction,
            text: &oversized,
            in_reply_to: None,
            report: None,
            stale: false,
            escalated_from: None,
        })
        .unwrap_err()
        .code,
        "message_too_large"
    );
    append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "first", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, escalated_from: None })
    .unwrap();
    state.messages[0].seq = u64::MAX;
    assert_eq!(
        append(&mut state, AppendMessage {
            from: ActorRef::Operator,
            to_run_id: "worker",
            message_id: "second",
            kind: MessageKind::Instruction,
            text: "hello",
            in_reply_to: None,
            report: None,
            stale: false,
            escalated_from: None,
        })
        .unwrap_err()
        .code,
        "message_sequence_exhausted"
    );
    assert_eq!(state.messages.len(), 1);
}

#[test]
fn initialized_and_result_use_registered_ownership_without_physical_proof() {
    let mut state = state();
    let offline = disconnected_actor(main_actor("worker"));
    let attachment = state.runs[1].attachment.as_mut().unwrap();
    attachment.process = None;
    attachment.state = AttachmentState::Disconnected;
    mutation(&mut state, &offline, 1, &report("initialized", ReportKind::Initialized, None));
    assert_eq!(state.runs[1].stage, RunStage::Working);
    mutation(&mut state, &offline, 1, &report("result", ReportKind::Result, None));
    assert_eq!(state.runs[1].stage, RunStage::Reported);
    assert_eq!(state.runs[1].result.as_ref().unwrap().message_id, "result");
}

#[test]
fn latest_progress_delivery_preserves_questions_and_durable_retry_ids() {
    let mut state = state();
    let worker = main_actor("worker");
    let root = main_actor("root");
    let old_progress = report("progress-old", ReportKind::Progress, None);
    mutation(&mut state, &worker, 1, &old_progress);
    mutation(&mut state, &worker, 1, &report("question", ReportKind::NeedsInput, None));
    mutation(&mut state, &worker, 1, &report("progress-new", ReportKind::Progress, None));
    let inbox = mutation(&mut state, &root, 0, &OrchestrationAction::InboxPull {
        after_seq: 0, limit: 100,
    });
    let OrchestrationActionResult::Inbox { messages, read_through_seq } = inbox else {
        unreachable!()
    };
    assert_eq!(messages.iter().map(|message| message.message_id.as_str()).collect::<Vec<_>>(),
        ["question", "progress-new"]);
    mutation(&mut state, &root, 0, &OrchestrationAction::InboxAck { through_seq: read_through_seq });
    assert_eq!(state.messages[0].stage, DeliveryStage::Stored);
    assert_eq!(state.runs[1].last_report.as_ref().unwrap().message_id, "progress-new");
    let before = serde_json::to_value(&state).unwrap();
    assert!(matches!(mutation(&mut state, &worker, 1, &old_progress),
        OrchestrationActionResult::Message { duplicate: true, .. }));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    apply(&mut state, &Actor::Operator(OperatorOrigin::Browser), None, false,
        &send("answer", MessageKind::Answer, Some("question"))).unwrap();
    assert_eq!(state.runs[1].stage, RunStage::Working);
    assert_eq!(state.messages.last().unwrap().in_reply_to.as_deref(), Some("question"));
}

#[test]
fn accepted_history_refuses_new_management_writes_but_keeps_message_replay() {
    let mut state = state();
    let operator = Actor::Operator(OperatorOrigin::Browser);
    let instruction = send("instruction", MessageKind::Instruction, None);
    apply(&mut state, &operator, None, false, &instruction).unwrap();
    state.runs[1].stage = RunStage::Closed;
    state.runs[1].close_reason = Some(CloseReason::Accepted);
    let before = serde_json::to_value(&state).unwrap();
    assert!(matches!(apply(&mut state, &operator, None, false, &instruction).unwrap(),
        Some(OrchestrationActionResult::Message { duplicate: true, .. })));
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    assert_rejected_without_mutation(&mut state,
        &send("new-instruction", MessageKind::Instruction, None), "invalid_stage");
    let annotation = OrchestrationAction::Annotate {
        run_id: "worker".into(), text: "Change accepted history".into(),
    };
    assert_rejected_without_mutation(&mut state, &annotation, "invalid_stage");
    assert_error(&mut state, &main_actor("root"), 0, &annotation, "invalid_stage");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

#[test]
fn committed_result_replays_after_state_reload_without_a_semantic_write() {
    let mut state = state();
    let worker = main_actor("worker");
    let result = report("result", ReportKind::Result, None);
    mutation(&mut state, &worker, 1, &result);
    let mut recovered: OrchestrationState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    let before = serde_json::to_value(&recovered).unwrap();
    assert!(matches!(mutation(&mut recovered, &worker, 1, &result),
        OrchestrationActionResult::Message { duplicate: true, .. }));
    assert_eq!(serde_json::to_value(&recovered).unwrap(), before);
    let mut changed = result;
    if let OrchestrationAction::Report { source_output, .. } = &mut changed {
        if let Some(SourceOutput::NonCode { artifacts, .. }) = source_output {
            artifacts.push("different-output".into());
        }
    }
    assert_error(&mut recovered, &worker, 1, &changed, "message_id_conflict");
}

#[test]
fn repeated_pull_and_explicit_ack_do_not_mutate_durable_receipts() {
    let mut state = state();
    let worker = main_actor("worker");
    apply(&mut state, &Actor::Operator(OperatorOrigin::Browser), None, false,
        &send("instruction", MessageKind::Instruction, None)).unwrap();
    let pull = OrchestrationAction::InboxPull { after_seq: 0, limit: 100 };
    mutation(&mut state, &worker, 1, &pull);
    let before = serde_json::to_value(&state).unwrap();
    mutation(&mut state, &worker, 1, &pull);
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    let ack = OrchestrationAction::InboxAck { through_seq: 1 };
    mutation(&mut state, &worker, 1, &ack);
    let before = serde_json::to_value(&state).unwrap();
    mutation(&mut state, &worker, 1, &ack);
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

#[test]
fn upward_reports_reject_self_siblings_descendants_and_escalate_closed_parent() {
    let mut state = state();
    let actor = main_actor("grandchild");
    for destination in ["grandchild", "sibling"] {
        assert_error(
            &mut state,
            &actor,
            2,
            &report("invalid", ReportKind::Progress, Some(destination)),
            "not_ancestor",
        );
    }
    assert_error(
        &mut state,
        &main_actor("worker"),
        1,
        &report("downward", ReportKind::Progress, Some("grandchild")),
        "not_ancestor",
    );
    mutation(
        &mut state,
        &actor,
        2,
        &report("root-directed", ReportKind::Progress, Some("root")),
    );
    state.runs[1].stage = RunStage::Closed;
    mutation(
        &mut state,
        &actor,
        2,
        &report("escalated", ReportKind::NeedsInput, None),
    );
    assert_eq!(state.messages[1].to_run_id, "root");
    assert_eq!(state.messages[1].escalated_from.as_deref(), Some("worker"));
    state.runs[0].stage = RunStage::Closed;
    let duplicate = mutation(
        &mut state,
        &actor,
        2,
        &report("escalated", ReportKind::NeedsInput, None),
    );
    assert!(matches!(
        duplicate,
        OrchestrationActionResult::Message {
            duplicate: true,
            seq: 2,
            ..
        }
    ));
}

#[test]
fn stale_epoch_and_closed_job_cannot_append_authoritative_evidence() {
    let mut state = state();
    let actor = main_actor("worker");
    state.runs[1].attachment.as_mut().unwrap().epoch = 2;
    let before = serde_json::to_value(&state).unwrap();
    assert_eq!(
        apply(&mut state, &actor, Some(1), true, &report("stale", ReportKind::Result, None))
            .unwrap_err().code,
        "job_unauthorized"
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    state.runs[1].attachment.as_mut().unwrap().epoch = 1;
    state.runs[1].stage = RunStage::Closed;
    state.runs[1].close_reason = Some(CloseReason::Superseded);
    let before = serde_json::to_value(&state).unwrap();
    assert_eq!(
        apply(&mut state, &actor, Some(1), true, &report("closed", ReportKind::Result, None))
            .unwrap_err().code,
        "job_unauthorized"
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

#[test]
fn root_reports_to_operator_visible_self_inbox() {
    let mut state = state();
    mutation(
        &mut state,
        &main_actor("root"),
        0,
        &report("need", ReportKind::NeedsInput, None),
    );
    mutation(
        &mut state,
        &main_actor("root"),
        0,
        &report("result", ReportKind::Result, None),
    );
    assert!(
        state
            .messages
            .iter()
            .all(|message| message.to_run_id == "root")
    );
    assert_eq!(state.runs[0].stage, RunStage::Active);
    assert!(state.runs[0].result.is_none());
    assert_eq!(state.runs[0].last_report.as_ref().unwrap().message_id, "result");
}


#[test]
fn durable_pull_and_ack_are_atomic_across_the_whole_prefix() {
    let mut state = state();
    let operator = Actor::Operator(OperatorOrigin::Browser);
    for id in ["first", "second"] {
        apply(
            &mut state,
            &operator,
            None,
            false,
            &OrchestrationAction::MessageSend {
                message_id: id.into(),
                to_run_id: "worker".into(),
                kind: MessageKind::Instruction,
                text: id.into(),
                in_reply_to: None,
                original_task_revision: None,
                original_claim_id: None,
            },
        )
        .unwrap();
    }
    let actor = main_actor("worker");
    assert_error(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::InboxAck { through_seq: 2 },
        "ack_before_read",
    );
    assert!(
        state
            .messages
            .iter()
            .all(|message| message.stage == DeliveryStage::Stored)
    );
    mutation(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::InboxPull {
            after_seq: 0,
            limit: 1,
        },
    );
    assert_error(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::InboxAck { through_seq: 2 },
        "ack_before_read",
    );
    assert_eq!(state.messages[0].stage, DeliveryStage::Read);
    mutation(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::InboxAck { through_seq: 1 },
    );
    let acked_at = state.messages[0].acked_at.clone();
    mutation(&mut state, &actor, 1, &OrchestrationAction::InboxAck { through_seq: 1 });
    assert_eq!(state.messages[0].stage, DeliveryStage::Acked);
    assert_eq!(state.messages[0].acked_at, acked_at);
    mutation(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::InboxPull {
            after_seq: 1,
            limit: 1,
        },
    );
    mutation(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::InboxAck { through_seq: 2 },
    );
    assert!(
        state
            .messages
            .iter()
            .all(|message| message.stage == DeliveryStage::Acked)
    );
}

#[test]
fn pull_caps_at_one_hundred_and_is_scoped_to_registered_run_inbox() {
    let mut state = state();
    for seq in 0..105 {
        append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: &format!("message-{seq}"), kind: MessageKind::Instruction, text: "read", in_reply_to: None, report: None, stale: false, escalated_from: None })
        .unwrap();
    }
    let result = mutation(
        &mut state,
        &main_actor("worker"),
        1,
        &OrchestrationAction::InboxPull {
            after_seq: 0,
            limit: u32::MAX,
        },
    );
    let OrchestrationActionResult::Inbox {
        messages,
        read_through_seq,
    } = result
    else {
        panic!("expected inbox");
    };
    assert_eq!(messages.len(), 100);
    assert_eq!(read_through_seq, 100);
    assert_eq!(state.messages[100].stage, DeliveryStage::Stored);
    let root_inbox = mutation(&mut state, &main_actor("root"), 0,
        &OrchestrationAction::InboxPull { after_seq: 0, limit: 100 });
    assert!(matches!(root_inbox, OrchestrationActionResult::Inbox { ref messages, read_through_seq: 0 } if messages.is_empty()));
}

#[test]
fn message_and_annotation_authority_is_directional_and_append_only() {
    let mut state = state();
    let actor = main_actor("worker");
    for (target, kind, expected) in [
        ("grandchild", MessageKind::Instruction, Some("actor_forbidden")),
        ("grandchild", MessageKind::CancelRequest, Some("actor_forbidden")),
        ("root", MessageKind::Observation, None),
        ("sibling", MessageKind::Instruction, Some("actor_forbidden")),
        ("root", MessageKind::Instruction, Some("actor_forbidden")),
        ("worker", MessageKind::Instruction, Some("actor_forbidden")),
        ("root", MessageKind::Answer, Some("actor_forbidden")),
    ] {
        let action = OrchestrationAction::MessageSend {
            message_id: format!("{target}-{kind:?}"),
            to_run_id: target.into(),
            kind,
            text: "payload".into(),
            in_reply_to: None,
            original_task_revision: None,
            original_claim_id: None,
        };
        if let Some(code) = expected {
            assert_error(&mut state, &actor, 1, &action, code);
        } else {
            mutation(&mut state, &actor, 1, &action);
        }
    }
    assert_eq!(state.runs[2].stage, RunStage::Working);
    for target in ["worker"] {
        mutation(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::Annotate {
                run_id: target.into(),
                text: "annotation".into(),
            },
        );
    }
    mutation(
        &mut state,
        &main_actor("root"),
        0,
        &OrchestrationAction::Annotate {
            run_id: "grandchild".into(), text: "root annotation".into(),
        },
    );
    assert_error(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::Annotate {
            run_id: "root".into(),
            text: "bad".into(),
        },
        "not_in_subtree",
    );
    assert_eq!(state.runs[1].annotations.len(), 1);
    assert_eq!(state.runs[2].annotations.len(), 1);
    let operator = Actor::Operator(OperatorOrigin::Native);
    assert_eq!(
        apply(
            &mut state,
            &operator,
            None,
            false,
            &OrchestrationAction::MessageSend {
                message_id: "operator-observation".into(),
                to_run_id: "root".into(),
                kind: MessageKind::Observation,
                text: "bad".into(),
                in_reply_to: None,
                original_task_revision: None,
                original_claim_id: None,
            }
        )
        .unwrap_err()
        .code,
        "actor_forbidden"
    );
}


fn disconnected_actor(actor: Actor) -> Actor {
    let Actor::Agent(mut caller) = actor else { unreachable!() };
    caller.endpoint_identity.clear();
    caller.session_id.clear();
    caller.workspace_id.clear();
    caller.tab_id.clear();
    caller.pane_id.clear();
    caller.actual_agent_kind = None;
    caller.process = None;
    Actor::Agent(caller)
}

#[test]
fn ordinary_messages_inbox_and_result_survive_disconnected_attachment() {
    let mut state = state();
    for run in &mut state.runs {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.state = AttachmentState::Disconnected;
        attachment.process = None;
        attachment.location = None;
    }
    let root = disconnected_actor(main_actor("root"));
    let worker = disconnected_actor(main_actor("worker"));
    mutation(&mut state, &root, 0, &send("offline-instruction", MessageKind::Instruction, None));
    let inbox = mutation(
        &mut state, &worker, 1,
        &OrchestrationAction::InboxPull { after_seq: 0, limit: 10 },
    );
    assert!(matches!(inbox, OrchestrationActionResult::Inbox { ref messages, read_through_seq: 1 } if messages.len() == 1));
    mutation(&mut state, &worker, 1, &OrchestrationAction::InboxAck { through_seq: 1 });
    assert_eq!(state.messages[0].stage, DeliveryStage::Acked);
    state.runs[1].stage = RunStage::Working;
    mutation(&mut state, &worker, 1, &report("offline-result", ReportKind::Result, None));
    assert_eq!(state.runs[1].stage, RunStage::Reported);
    assert_eq!(state.runs[1].result.as_ref().unwrap().message_id, "offline-result");
}

#[test]
fn invalid_credentials_and_role_forgery_leave_messages_unchanged() {
    let mut state = state();
    let Actor::Agent(worker) = main_actor("worker") else { unreachable!() };
    let action = report("bad-auth", ReportKind::Progress, None);
    for mutation_kind in ["unknown", "secret", "epoch", "attachment"] {
        let mut caller = worker.clone();
        match mutation_kind {
            "unknown" => caller.auth.credential_id = uuid::Uuid::new_v4().to_string(),
            "secret" => caller.auth.secret = SecretBytes::new([0x19; 32]),
            "epoch" => caller.auth.attachment_epoch = 2,
            _ => caller.auth.attachment_id = uuid::Uuid::new_v4().to_string(),
        }
        let before = serde_json::to_value(&state).unwrap();
        assert_error(&mut state, &Actor::Agent(caller), 1, &action, "job_unauthorized");
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
    let forged_root = worker;
    let before = serde_json::to_value(&state).unwrap();
    assert_error(
        &mut state, &Actor::Agent(forged_root), 1,
        &OrchestrationAction::MessageSend {
            message_id: "forged-root".into(), to_run_id: "grandchild".into(),
            kind: MessageKind::Instruction, text: "forbidden".into(), in_reply_to: None,
            original_task_revision: None, original_claim_id: None,
        },
        "actor_forbidden",
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    let root_record = state.credentials.iter_mut().find(|record| record.run_id == "worker" && record.role == JobRole::Worker).unwrap();
    root_record.role = JobRole::Root;
    let before = serde_json::to_value(&state).unwrap();
    assert_error(&mut state, &main_actor("worker"), 1, &action, "job_unauthorized");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}


#[test]
fn conversation_and_assignment_generation_fence_messages_and_ack() {
    let mut state = state();
    let Actor::Agent(worker) = main_actor("worker") else { unreachable!() };
    apply(&mut state, &Actor::Operator(OperatorOrigin::Browser), None, false,
        &send("instruction", MessageKind::Instruction, None)).unwrap();
    mutation(&mut state, &Actor::Agent(worker.clone()), 1,
        &OrchestrationAction::InboxPull { after_seq: 0, limit: 10 });
    for mismatch in ["harness", "conversation", "generation", "owner"] {
        let mut candidate = state.clone();
        let mut caller = worker.clone();
        match mismatch {
            "harness" => caller.conversation.as_mut().unwrap().harness = "other-harness".into(),
            "conversation" => caller.conversation.as_mut().unwrap().session_id = "another-chat".into(),
            "generation" => caller.assignment_generation = Some(2),
            _ => candidate.task_ownership.iter_mut()
                .find(|record| record.task_id == "task-worker").unwrap().owner = None,
        }
        for action in [
            report("unauthorized-progress", ReportKind::Progress, None),
            OrchestrationAction::InboxAck { through_seq: 1 },
        ] {
            let before = serde_json::to_value(&candidate).unwrap();
            assert_error(&mut candidate, &Actor::Agent(caller.clone()), 1, &action, "job_unauthorized");
            assert_eq!(serde_json::to_value(&candidate).unwrap(), before);
        }
    }
}

#[test]
fn stored_inbox_survives_reload_and_ack_captures_without_completing_work() {
    let mut state = state();
    let operator = Actor::Operator(OperatorOrigin::Browser);
    apply(&mut state, &operator, None, false, &send("durable-instruction", MessageKind::Instruction, None)).unwrap();
    let mut recovered: OrchestrationState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(recovered.messages[0].stage, DeliveryStage::Stored);
    let actor = main_actor("worker");
    let inbox = mutation(&mut recovered, &actor, 1,
        &OrchestrationAction::InboxPull { after_seq: 0, limit: 10 });
    assert!(matches!(inbox, OrchestrationActionResult::Inbox { ref messages, read_through_seq: 1 }
        if messages.len() == 1 && messages[0].message_id == "durable-instruction"));
    mutation(&mut recovered, &actor, 1, &OrchestrationAction::InboxAck { through_seq: 1 });
    assert_eq!(recovered.messages[0].stage, DeliveryStage::Acked);
    assert_eq!(recovered.runs[1].stage, RunStage::Working);
    assert!(recovered.runs[1].result.is_none());
    assert_eq!(recovered.task_ownership[0].generation, 1);
    let before = serde_json::to_value(&recovered).unwrap();
    let reread = mutation(&mut recovered, &actor, 1,
        &OrchestrationAction::InboxPull { after_seq: 0, limit: 10 });
    assert!(matches!(reread, OrchestrationActionResult::Inbox { ref messages, .. }
        if messages.len() == 1 && messages[0].text == "Proceed with the approved choice"));
    assert_eq!(serde_json::to_value(&recovered).unwrap(), before);
}

#[test]
fn historical_message_replay_cannot_restore_revoked_task_ownership() {
    let mut state = state();
    let actor = main_actor("worker");
    let progress = report("committed-progress", ReportKind::Progress, None);
    mutation(&mut state, &actor, 1, &progress);
    let ownership = state.task_ownership.iter_mut()
        .find(|record| record.task_id == "task-worker").unwrap();
    ownership.generation += 1;
    ownership.owner = None;
    let before = serde_json::to_value(&state).unwrap();
    assert_error(&mut state, &actor, 1, &progress, "job_unauthorized");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    assert_eq!(state.messages.len(), 1);
}
