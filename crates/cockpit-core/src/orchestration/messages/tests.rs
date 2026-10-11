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

fn fixture_auth(id: &str, subject: Option<&str>) -> JobAuth {
    let parent = (id != "root").then_some("root");
    let run = run(id, parent, RunStage::Working);
    let role = if subject.is_some() {
        JobRole::Child
    } else if parent.is_some() {
        JobRole::Worker
    } else {
        JobRole::Root
    };
    test_job_credential(&run, role, subject).1
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
            main_session_id: Some(format!("omp-{id}")),
            process: Some(native_process()),
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
        schema: 2,
        revision: 0,
        runs: vec![
            run("root", None, RunStage::Active),
            run("worker", Some("root"), RunStage::Working),
            run("grandchild", Some("worker"), RunStage::Working),
            run("sibling", Some("root"), RunStage::Working),
        ],
        messages: Vec::new(),
        subagents: Vec::new(),
        task_intents: Vec::new(),
        assignment_intents: Vec::new(),
        ..OrchestrationState::default()
    };
    for index in 0..state.runs.len() {
        let role = if index == 0 { JobRole::Root } else { JobRole::Worker };
        test_job_auth(&mut state, index, role, None);
    }
    for subject in ["child", "wrong-child"] {
        test_job_auth(&mut state, 1, JobRole::Child, Some(subject));
    }
    state
}

fn main_actor(run_id: &str) -> Actor {
    Actor::Agent(AgentCaller {
        auth: fixture_auth(run_id, None),
        provenance: None,
        endpoint_identity: "endpoint".into(),
        session_id: "herdr-session".into(),
        workspace_id: "space".into(),
        tab_id: "tab".into(),
        pane_id: "pane".into(),
        boot_id: None,
        terminal_id: None,
        native_session_id: None,
        env_run: Some((run_id.into(), 1)),
        omp_session_id: Some(format!("omp-{run_id}")),
        main_omp_session_id: Some(format!("omp-{run_id}")),
        agent_kind: Some(AgentKind::Main),
        actual_agent_kind: Some("omp".into()),
        subagent_id: None,
        process: Some(native_process()),
    })
}

fn subagent_actor(run_id: &str, subagent_id: &str) -> Actor {
    let Actor::Agent(mut caller) = main_actor(run_id) else {
        unreachable!()
    };
    caller.agent_kind = Some(AgentKind::Subagent);
    caller.subagent_id = Some(subagent_id.into());
    caller.auth = fixture_auth(run_id, Some(subagent_id));
    caller.omp_session_id = Some(format!("sub-session-{subagent_id}"));
    Actor::Agent(caller)
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

fn update(id: &str, parent: Option<&str>) -> OrchestrationAction {
    OrchestrationAction::SubagentUpdate {
        subagent_id: id.into(),
        parent_subagent_id: parent.map(str::to_owned),
        role: Some("reviewer".into()),
        label: id.into(),
        status: SubagentStatus::Running,
        summary: None,
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
fn subagent_and_stale_questions_cannot_replace_the_main_question() {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    mutation(
        &mut state,
        &main_actor("worker"),
        1,
        &report("main-question", ReportKind::NeedsInput, None),
    );
    mutation(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &report("child-question", ReportKind::NeedsInput, Some("worker")),
    );
    assert_eq!(apply(
        &mut state,
        &main_actor("worker"),
        Some(1),
        true,
        &report("stale-question", ReportKind::NeedsInput, None),
    ).unwrap_err().code, "attempt_stale");
    for question in ["child-question", "stale-question"] {
        assert_rejected_without_mutation(
            &mut state,
            &send("answer", MessageKind::Answer, Some(question)),
            "question_not_current",
        );
    }
    apply(
        &mut state,
        &Actor::Operator(OperatorOrigin::Browser),
        None,
        false,
        &send("answer", MessageKind::Answer, Some("main-question")),
    ).unwrap();
    assert_eq!(state.runs[1].last_report.as_ref().unwrap().message_id, "main-question");
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
    let first = append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "same", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
    .unwrap();
    assert!(matches!(
        first,
        OrchestrationActionResult::Message {
            seq: 1,
            duplicate: false,
            ..
        }
    ));
    let duplicate = append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "same", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
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
    }, to_run_id: "worker", message_id: "same", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
    .unwrap();
    append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "sibling", message_id: "other", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
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
            from_subagent_id: None,
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
            from_subagent_id: None,
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
            from_subagent_id: None,
            escalated_from: None,
        })
        .unwrap_err()
        .code,
        "message_too_large"
    );
    append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "first", kind: MessageKind::Instruction, text: "hello", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
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
            from_subagent_id: None,
            escalated_from: None,
        })
        .unwrap_err()
        .code,
        "message_sequence_exhausted"
    );
    assert_eq!(state.messages.len(), 1);
}

#[test]
fn initialized_requires_main_but_does_not_gate_work() {
    let mut state = state();
    let initialized = report("initialized", ReportKind::Initialized, None);
    assert_error(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &initialized,
        "report_requires_main",
    );
    let Actor::Agent(mut offline) = main_actor("worker") else {
        unreachable!()
    };
    offline.omp_session_id = Some("new-conversation".into());
    offline.main_omp_session_id = None;
    offline.process = None;
    offline.provenance = None;
    let attachment = state.runs[1].attachment.as_mut().unwrap();
    attachment.main_session_id = None;
    attachment.process = None;
    attachment.state = AttachmentState::Disconnected;
    mutation(&mut state, &Actor::Agent(offline), 1, &initialized);
    assert_eq!(state.runs[1].stage, RunStage::Working);
    mutation(&mut state, &main_actor("worker"), 1, &report("result", ReportKind::Result, None));
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
fn unchanged_telemetry_and_transport_receipts_do_not_mutate_state() {
    let mut state = state();
    let worker = main_actor("worker");
    let telemetry = update("child", None);
    mutation(&mut state, &worker, 1, &telemetry);
    state.subagents[0].updated_at = "original-update".into();
    let before = serde_json::to_value(&state).unwrap();
    mutation(&mut state, &worker, 1, &telemetry);
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    apply(&mut state, &Actor::Operator(OperatorOrigin::Browser), None, false,
        &send("instruction", MessageKind::Instruction, None)).unwrap();
    let wake = OrchestrationAction::InboxWoken {
        through_seq: 1, omp_session_id: "native-worker".into(),
    };
    mutation(&mut state, &worker, 1, &wake);
    let before = serde_json::to_value(&state).unwrap();
    mutation(&mut state, &worker, 1, &wake);
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
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
    assert_eq!(state.runs[0].result.as_ref().unwrap().message_id, "result");
}

#[test]
fn subagent_progress_is_tagged_without_overwriting_main_receipt() {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    mutation(
        &mut state,
        &main_actor("worker"),
        1,
        &report("main", ReportKind::Progress, None),
    );
    mutation(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &report("child", ReportKind::NeedsInput, None),
    );
    assert_eq!(state.messages[1].from_subagent_id.as_deref(), Some("child"));
    assert_eq!(
        state.runs[1].last_report.as_ref().unwrap().message_id,
        "main"
    );
}

#[test]
fn internal_subagent_reports_to_own_main_are_evidence_only_and_require_real_child_identity() {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    mutation(
        &mut state,
        &main_actor("worker"),
        1,
        &report("main", ReportKind::Progress, None),
    );
    let main_before = serde_json::to_value(&state.runs[1]).unwrap();
    let child = subagent_actor("worker", "child");
    for (id, kind) in [
        ("child-progress", ReportKind::Progress),
        ("child-question", ReportKind::NeedsInput),
    ] {
        let action = report(id, kind, Some("worker"));
        let result = mutation(&mut state, &child, 1, &action);
        assert!(
            matches!(result, OrchestrationActionResult::Message { ref to_run_id, stale: false, .. } if to_run_id == "worker")
        );
        let message = state.messages.last().unwrap();
        assert_eq!(message.from_subagent_id.as_deref(), Some("child"));
        assert_eq!(message.to_run_id, "worker");
        assert_eq!(message.stage, DeliveryStage::Stored);
        assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), main_before);
    }
    assert_error(
        &mut state,
        &main_actor("worker"),
        1,
        &report("main-self", ReportKind::Progress, Some("worker")),
        "not_ancestor",
    );
    assert_error(
        &mut state,
        &child,
        1,
        &report("child-initialized", ReportKind::Initialized, Some("worker")),
        "report_requires_main",
    );
    assert_error(
        &mut state,
        &child,
        1,
        &report("child-result", ReportKind::Result, Some("worker")),
        "report_requires_main",
    );
    assert_error(
        &mut state,
        &child,
        1,
        &report("child-sibling", ReportKind::Progress, Some("sibling")),
        "not_ancestor",
    );
    let Actor::Agent(mut forged_main) = child.clone() else {
        unreachable!()
    };
    forged_main.agent_kind = Some(AgentKind::Main);
    forged_main.subagent_id = None;
    forged_main.omp_session_id = Some("omp-worker".into());
    assert_error(
        &mut state,
        &Actor::Agent(forged_main.clone()),
        1,
        &report("forged-result", ReportKind::Result, Some("worker")),
        "report_requires_main",
    );
    mutation(
        &mut state,
        &Actor::Agent(forged_main),
        1,
        &report("credential-subject", ReportKind::Progress, Some("worker")),
    );
    assert_eq!(state.messages.last().unwrap().from_subagent_id.as_deref(), Some("child"));
    assert_eq!(state.messages.len(), 4);
    assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), main_before);
}

#[test]
fn wake_never_reads_and_ack_is_atomic_across_the_whole_prefix() {
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
    let wake = OrchestrationAction::InboxWoken {
        through_seq: 2,
        omp_session_id: "omp-worker".into(),
    };
    mutation(&mut state, &actor, 1, &wake);
    mutation(&mut state, &actor, 1, &wake);
    assert!(
        state
            .messages
            .iter()
            .all(|message| message.stage == DeliveryStage::Woken)
    );
    assert_error(
        &mut state,
        &actor,
        1,
        &OrchestrationAction::InboxAck { through_seq: 2 },
        "ack_before_read",
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
    mutation(&mut state, &actor, 1, &wake);
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
fn pull_caps_at_one_hundred_and_is_scoped_to_main_inbox() {
    let mut state = state();
    for seq in 0..105 {
        append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: &format!("message-{seq}"), kind: MessageKind::Instruction, text: "read", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
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
    assert_error(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &OrchestrationAction::InboxPull {
            after_seq: 0,
            limit: 1,
        },
        "actor_forbidden",
    );
    assert_error(
        &mut state,
        &main_actor("worker"),
        1,
        &OrchestrationAction::InboxWoken {
            through_seq: 1,
            omp_session_id: "".into(),
        },
        "invalid_session",
    );
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
        (
            "grandchild",
            MessageKind::SubagentControl,
            Some("actor_forbidden"),
        ),
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

#[test]
fn telemetry_upserts_validates_nesting_and_scopes_subagent_identity() {
    let mut state = state();
    let main = main_actor("worker");
    mutation(&mut state, &main, 1, &update("parent", None));
    mutation(&mut state, &main, 1, &update("child", None));
    mutation(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &update("child", Some("parent")),
    );
    mutation(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &update("child", Some("parent")),
    );
    assert_eq!(state.subagents.len(), 2);
    assert_error(
        &mut state,
        &main,
        1,
        &update("parent", Some("child")),
        "invalid_subagent_parent",
    );
    assert_error(
        &mut state,
        &main,
        1,
        &update("self", Some("self")),
        "invalid_subagent_parent",
    );
    assert_error(
        &mut state,
        &main,
        1,
        &update("unknown", Some("absent")),
        "invalid_subagent_parent",
    );
    mutation(
        &mut state,
        &main_actor("sibling"),
        3,
        &update("foreign", None),
    );
    assert_error(
        &mut state,
        &main,
        1,
        &update("crossrun", Some("foreign")),
        "invalid_subagent_parent",
    );
    assert_error(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &update("parent", None),
        "actor_forbidden",
    );
    assert_eq!(state.subagents[0].parent_subagent_id, None);
}

#[test]
fn only_authenticated_self_updates_bind_native_children() {
    let mut state = state();
    let main = main_actor("worker");
    mutation(&mut state, &main, 1, &update("child", None));
    assert_eq!(state.subagents[0].bound_omp_session, None);

    let child = subagent_actor("worker", "child");
    mutation(&mut state, &child, 1, &update("child", None));
    assert_eq!(
        state.subagents[0].bound_omp_session.as_deref(),
        Some("sub-session-child")
    );
    mutation(&mut state, &main, 1, &update("child", None));
    assert_eq!(
        state.subagents[0].bound_omp_session.as_deref(),
        Some("sub-session-child")
    );

    let Actor::Agent(mut unproven) = child else { unreachable!() };
    unproven.process = None;
    mutation(&mut state, &Actor::Agent(unproven.clone()), 1, &update("child", None));
    unproven.process = Some(NativeProcessIdentity {
        start_ticks: 74,
        ..native_process()
    });
    mutation(&mut state, &Actor::Agent(unproven.clone()), 1, &update("child", None));
    unproven.process = Some(native_process());
    unproven.actual_agent_kind = Some("shell".into());
    mutation(&mut state, &Actor::Agent(unproven), 1, &update("child", None));
    assert_eq!(
        state.subagents[0].bound_omp_session.as_deref(),
        Some("sub-session-child")
    );
}

#[test]
fn lifecycle_updates_preserve_assignment_and_authoritative_child_progress() {
    let mut state = state();
    let main = main_actor("worker");
    let mut assignment = update("child", None);
    let OrchestrationAction::SubagentUpdate { summary, .. } = &mut assignment else {
        unreachable!()
    };
    *summary = Some("Inspect README and report findings".into());
    mutation(&mut state, &main, 1, &assignment);
    mutation(&mut state, &main, 1, &update("child", None));
    assert_eq!(
        state.subagents[0].summary.as_deref(),
        Some("Inspect README and report findings")
    );
    let main_before = serde_json::to_value(&state.runs[1]).unwrap();
    mutation(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &report("child-finding", ReportKind::Progress, None),
    );
    assert_eq!(
        state.subagents[0].summary.as_deref(),
        Some("summary-child-finding")
    );
    let mut terminal = update("child", None);
    let OrchestrationAction::SubagentUpdate { status, .. } = &mut terminal else {
        unreachable!()
    };
    *status = SubagentStatus::Done;
    mutation(&mut state, &main, 1, &terminal);
    assert_eq!(state.subagents[0].status, SubagentStatus::Done);
    assert_eq!(
        state.subagents[0].summary.as_deref(),
        Some("summary-child-finding")
    );
    mutation(&mut state, &main, 1, &assignment);
    assert_eq!(
        state.subagents[0].summary.as_deref(),
        Some("summary-child-finding")
    );
    assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), main_before);
}

#[test]
fn controls_encode_real_operation_and_done_acks_only_target_sequence() {
    let mut state = state();
    let main = main_actor("worker");
    mutation(&mut state, &main, 1, &update("child", None));
    append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "brief", kind: MessageKind::Instruction, text: "read first", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
    .unwrap();
    let control = OrchestrationAction::SubagentControl {
        run_id: "worker".into(),
        subagent_id: "child".into(),
        op: SubagentOp::Send {
            text: "actual payload".into(),
        },
    };
    let receipt = apply(
        &mut state,
        &Actor::Operator(OperatorOrigin::Browser),
        None,
        false,
        &control,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        receipt,
        OrchestrationActionResult::Message { seq: 2, .. }
    ));
    let envelope: serde_json::Value = serde_json::from_str(&state.messages[1].text).unwrap();
    assert_eq!(
        envelope,
        serde_json::json!({ "subagent_id": "child", "op": { "op": "send", "text": "actual payload" } })
    );
    assert_eq!(
        state.subagents[0].last_control.as_ref().unwrap().stage,
        ControlStage::Stored
    );
    let done = OrchestrationAction::SubagentControlDone {
        seq: 2,
        applied: true,
        error: None,
    };
    assert_error(
        &mut state,
        &subagent_actor("worker", "wrong-child"),
        1,
        &done,
        "actor_forbidden",
    );
    assert_error(
        &mut state,
        &main_actor("root"),
        0,
        &done,
        "subagent_control_not_found",
    );
    mutation(&mut state, &subagent_actor("worker", "child"), 1, &done);
    assert_eq!(state.messages[0].stage, DeliveryStage::Stored);
    assert_eq!(state.messages[1].stage, DeliveryStage::Acked);
    assert_eq!(state.subagents[0].status, SubagentStatus::Running);
    assert_eq!(
        state.subagents[0].last_control.as_ref().unwrap().stage,
        ControlStage::Applied
    );
    mutation(&mut state, &main, 1, &done);
    assert_error(
        &mut state,
        &main,
        1,
        &OrchestrationAction::SubagentControlDone {
            seq: 2,
            applied: false,
            error: Some("late failure".into()),
        },
        "subagent_control_conflict",
    );
}

#[test]
fn ordinary_inbox_consumers_cannot_complete_or_starve_pending_controls() {
    let mut state = state();
    let main = main_actor("worker");
    mutation(&mut state, &main, 1, &update("child", None));
    mutation(&mut state, &subagent_actor("worker", "child"), 1, &update("child", None));
    let operator = Actor::Operator(OperatorOrigin::Browser);
    for _ in 0..100 {
        apply(
            &mut state,
            &operator,
            None,
            false,
            &OrchestrationAction::SubagentControl {
                run_id: "worker".into(),
                subagent_id: "child".into(),
                op: SubagentOp::Cancel,
            },
        )
        .unwrap();
    }
    append(&mut state, AppendMessage { from: ActorRef::Operator, to_run_id: "worker", message_id: "ordinary", kind: MessageKind::Instruction, text: "Handle this ordinary message", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })
    .unwrap();
    mutation(
        &mut state,
        &main,
        1,
        &OrchestrationAction::InboxWoken {
            through_seq: 101,
            omp_session_id: "omp-worker".into(),
        },
    );
    assert!(
        state.messages[..100]
            .iter()
            .all(|message| message.stage == DeliveryStage::Stored)
    );
    let result = mutation(
        &mut state,
        &main,
        1,
        &OrchestrationAction::InboxPull {
            after_seq: 0,
            limit: 1,
        },
    );
    let OrchestrationActionResult::Inbox {
        messages,
        read_through_seq,
    } = result
    else {
        panic!("expected inbox");
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].message_id, "ordinary");
    assert_eq!(read_through_seq, 101);
    mutation(
        &mut state,
        &main,
        1,
        &OrchestrationAction::InboxAck { through_seq: 101 },
    );
    assert_eq!(state.messages[100].stage, DeliveryStage::Acked);
    assert!(
        state.messages[..100]
            .iter()
            .all(|message| message.stage == DeliveryStage::Stored)
    );
    assert_eq!(
        state.subagents[0].last_control.as_ref().unwrap().stage,
        ControlStage::Stored
    );
    mutation(
        &mut state,
        &subagent_actor("worker", "child"),
        1,
        &OrchestrationAction::SubagentControlDone {
            seq: 100,
            applied: true,
            error: None,
        },
    );
    assert_eq!(state.messages[99].stage, DeliveryStage::Acked);
    assert!(
        state.messages[..99]
            .iter()
            .all(|message| message.stage == DeliveryStage::Stored)
    );
    assert_eq!(
        state.subagents[0].last_control.as_ref().unwrap().stage,
        ControlStage::Applied
    );
}

#[test]
fn control_requires_operator_or_strict_ancestor_and_retains_newer_receipt() {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    let action = OrchestrationAction::SubagentControl {
        run_id: "worker".into(),
        subagent_id: "child".into(),
        op: SubagentOp::Cancel,
    };
    assert_error(
        &mut state,
        &main_actor("worker"),
        1,
        &action,
        "actor_forbidden",
    );
    assert_error(
        &mut state,
        &main_actor("sibling"),
        3,
        &action,
        "actor_forbidden",
    );
    mutation(&mut state, &main_actor("root"), 0, &action);
    mutation(&mut state, &main_actor("root"), 0, &action);
    mutation(
        &mut state,
        &main_actor("worker"),
        1,
        &OrchestrationAction::SubagentControlDone {
            seq: 1,
            applied: true,
            error: None,
        },
    );
    assert_eq!(state.subagents[0].last_control.as_ref().unwrap().seq, 2);
    assert_eq!(
        state.subagents[0].last_control.as_ref().unwrap().stage,
        ControlStage::Stored
    );
    mutation(
        &mut state,
        &main_actor("worker"),
        1,
        &OrchestrationAction::SubagentControlDone {
            seq: 2,
            applied: false,
            error: Some("abort unavailable".into()),
        },
    );
    assert_eq!(
        state.subagents[0].last_control.as_ref().unwrap().stage,
        ControlStage::Failed
    );
    assert_eq!(state.subagents[0].status, SubagentStatus::Cancelled);
}

#[test]
fn child_updates_work_offline_but_cannot_manage_runs_or_other_children() {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    let Actor::Agent(mut child) = subagent_actor("worker", "child") else {
        unreachable!()
    };
    child.main_omp_session_id = None;
    child.omp_session_id = None;
    child.process = None;
    child.provenance = None;
    child.subagent_id = Some("forged-other-child".into());
    let child = Actor::Agent(child);
    mutation(&mut state, &child, 1, &update("child", None));
    mutation(&mut state, &child, 1, &report("offline-progress", ReportKind::Progress, None));
    assert_eq!(state.messages[0].from_subagent_id.as_deref(), Some("child"));
    assert_eq!(state.subagents[0].bound_omp_session, None);
    let before = serde_json::to_value(&state).unwrap();
    for action in [
        update("forged-other-child", None),
        OrchestrationAction::Annotate { run_id: "worker".into(), text: "forbidden".into() },
        OrchestrationAction::MessageSend {
            message_id: "child-instruction".into(), to_run_id: "grandchild".into(),
            kind: MessageKind::Instruction, text: "forbidden".into(), in_reply_to: None,
            original_task_revision: None, original_claim_id: None,
        },
        OrchestrationAction::SubagentControl {
            run_id: "grandchild".into(), subagent_id: "child".into(), op: SubagentOp::Cancel,
        },
    ] {
        assert_error(&mut state, &child, 1, &action, "actor_forbidden");
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
}

#[test]
fn main_telemetry_uses_job_authority_not_conversation_context() {
    let mut state = state();
    for actual_session in [None, Some("different-main-session")] {
        let Actor::Agent(mut caller) = main_actor("worker") else {
            unreachable!()
        };
        caller.omp_session_id = actual_session.map(str::to_owned);
        mutation(&mut state, &Actor::Agent(caller), 1, &update("child", None));
    }
    let actor = main_actor("worker");
    mutation(&mut state, &actor, 1, &update("direct", None));
    mutation(&mut state, &actor, 1, &update("nested", Some("direct")));
    assert_eq!(
        state.subagents.iter().find(|child| child.subagent_id == "nested").unwrap().parent_subagent_id.as_deref(),
        Some("direct")
    );
}

#[test]
fn terminal_cancel_receipt_requires_the_registered_child_session_before_ack() {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    mutation(&mut state, &subagent_actor("worker", "child"), 1, &update("child", None));
    mutation(
        &mut state,
        &main_actor("root"),
        0,
        &OrchestrationAction::SubagentControl {
            run_id: "worker".into(),
            subagent_id: "child".into(),
            op: SubagentOp::Cancel,
        },
    );
    let Actor::Agent(mut caller) = subagent_actor("worker", "child") else {
        unreachable!()
    };
    caller.omp_session_id = None;
    assert_error(
        &mut state,
        &Actor::Agent(caller),
        1,
        &OrchestrationAction::SubagentControlDone {
            seq: 1,
            applied: true,
            error: None,
        },
        "session_mismatch",
    );
    assert_eq!(state.messages[0].stage, DeliveryStage::Stored);
    assert_eq!(
        state.subagents[0].last_control.as_ref().unwrap().stage,
        ControlStage::Stored
    );
}

fn disconnected_actor(actor: Actor) -> Actor {
    let Actor::Agent(mut caller) = actor else { unreachable!() };
    caller.endpoint_identity.clear();
    caller.session_id.clear();
    caller.workspace_id.clear();
    caller.tab_id.clear();
    caller.pane_id.clear();
    caller.env_run = None;
    caller.omp_session_id = None;
    caller.main_omp_session_id = None;
    caller.agent_kind = None;
    caller.actual_agent_kind = None;
    caller.subagent_id = None;
    caller.process = None;
    caller.provenance = None;
    Actor::Agent(caller)
}

#[test]
fn ordinary_messages_inbox_and_result_survive_disconnected_attachment() {
    let mut state = state();
    for run in &mut state.runs {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.state = AttachmentState::Disconnected;
        attachment.main_session_id = None;
        attachment.process = None;
        attachment.location = None;
    }
    let root = disconnected_actor(main_actor("root"));
    let worker = disconnected_actor(main_actor("worker"));
    mutation(&mut state, &root, 0, &send("offline-instruction", MessageKind::Instruction, None));
    mutation(
        &mut state, &worker, 1,
        &OrchestrationAction::InboxWoken {
            through_seq: 1, omp_session_id: "new-conversation".into(),
        },
    );
    let inbox = mutation(
        &mut state, &worker, 1,
        &OrchestrationAction::InboxPull { after_seq: 0, limit: 10 },
    );
    assert!(matches!(inbox, OrchestrationActionResult::Inbox { ref messages, read_through_seq: 1 } if messages.len() == 1));
    mutation(&mut state, &worker, 1, &OrchestrationAction::InboxAck { through_seq: 1 });
    assert_eq!(state.messages[0].stage, DeliveryStage::Acked);
    assert_eq!(state.messages[0].woken_omp_session.as_deref(), Some("new-conversation"));
    mutation(&mut state, &worker, 1, &update("child", None));
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
    let mut forged_root = worker;
    forged_root.env_run = Some(("root".into(), 1));
    forged_root.omp_session_id = Some("omp-root".into());
    forged_root.main_omp_session_id = Some("omp-root".into());
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
fn native_control_still_requires_current_physical_main_and_child_evidence() {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    let control = OrchestrationAction::SubagentControl {
        run_id: "worker".into(), subagent_id: "child".into(), op: SubagentOp::Send { text: "native effect".into() },
    };
    let before = serde_json::to_value(&state).unwrap();
    assert_error(
        &mut state, &disconnected_actor(main_actor("root")), 0, &control, "session_mismatch",
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    mutation(&mut state, &main_actor("root"), 0, &control);
    let done = OrchestrationAction::SubagentControlDone { seq: 1, applied: true, error: None };
    for actor in [
        disconnected_actor(main_actor("worker")),
        disconnected_actor(subagent_actor("worker", "child")),
    ] {
        let before = serde_json::to_value(&state).unwrap();
        assert_error(&mut state, &actor, 1, &done, "session_mismatch");
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
    mutation(&mut state, &subagent_actor("worker", "child"), 1, &done);
    assert_eq!(state.messages[0].stage, DeliveryStage::Acked);
}

#[test]
fn main_credentials_cannot_authorize_explicit_child_contexts() {
    let mut state = state();
    for run_id in ["root", "worker"] {
        let index = if run_id == "root" { 0 } else { 1 };
        for source in ["kind", "subject", "provenance-kind", "provenance-subject"] {
            let Actor::Agent(mut caller) = main_actor(run_id) else { unreachable!() };
            match source {
                "kind" => caller.agent_kind = Some(AgentKind::Subagent),
                "subject" => caller.subagent_id = Some("child".into()),
                _ => caller.provenance = Some(super::super::CallerProvenance {
                    agent_kind: (source == "provenance-kind").then_some(AgentKind::Subagent),
                    child_id: (source == "provenance-subject").then(|| "child".into()),
                    main_session_id: None, native_session_id: None, process: None,
                }),
            }
            let before = serde_json::to_value(&state).unwrap();
            assert_error(
                &mut state, &Actor::Agent(caller), index,
                &report("inherited-main-token", ReportKind::Progress, None), "actor_forbidden",
            );
            assert_eq!(serde_json::to_value(&state).unwrap(), before);
        }
    }
    let bootstrap = Actor::Bootstrap(super::super::BootstrapCaller {
        endpoint_identity: "endpoint".into(), session_id: "herdr-session".into(),
        workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(),
        boot_id: None, terminal_id: None, native_session_id: None,
        omp_session_id: Some("omp-worker".into()), agent_kind: Some(AgentKind::Main),
        actual_agent_kind: Some("omp".into()), process: Some(native_process()),
    });
    let before = serde_json::to_value(&state).unwrap();
    assert_error(
        &mut state, &bootstrap, 1, &report("bootstrap-report", ReportKind::Progress, None),
        "actor_forbidden",
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

fn terminal_cancel_fixture() -> (OrchestrationState, Actor) {
    let mut state = state();
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    mutation(&mut state, &subagent_actor("worker", "child"), 1, &update("child", None));
    mutation(
        &mut state, &main_actor("root"), 0,
        &OrchestrationAction::SubagentControl {
            run_id: "worker".into(), subagent_id: "child".into(), op: SubagentOp::Cancel,
        },
    );
    let Actor::Agent(mut child) = subagent_actor("worker", "child") else { unreachable!() };
    child.process = None;
    child.actual_agent_kind = None;
    child.main_omp_session_id = Some("cached-old-main-conversation".into());
    child.provenance = None;
    (state, Actor::Agent(child))
}

#[test]
fn disposed_child_can_only_finish_its_registered_durable_cancel() {
    let (mut state, child) = terminal_cancel_fixture();
    let done = OrchestrationAction::SubagentControlDone { seq: 1, applied: true, error: None };
    mutation(&mut state, &child, 1, &done);
    assert_eq!(state.messages[0].stage, DeliveryStage::Acked);
    assert_eq!(state.subagents[0].status, SubagentStatus::Cancelled);
    assert_eq!(state.subagents[0].last_control.as_ref().unwrap().stage, ControlStage::Applied);
    mutation(&mut state, &child, 1, &done);
    let before = serde_json::to_value(&state).unwrap();
    assert_error(&mut state, &child, 1, &update("child", None), "invalid_stage");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    for kind in [ReportKind::Progress, ReportKind::NeedsInput] {
        assert_error(&mut state, &child, 1, &report("cancelled-report", kind, None), "invalid_stage");
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
    assert_error(
        &mut state, &child, 1,
        &OrchestrationAction::MessageSend {
            message_id: "cancelled-observation".into(), to_run_id: "root".into(),
            kind: MessageKind::Observation, text: "forbidden".into(), in_reply_to: None,
            original_task_revision: None, original_claim_id: None,
        },
        "invalid_stage",
    );
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    let mut cancelled = update("child", None);
    let OrchestrationAction::SubagentUpdate { status, .. } = &mut cancelled else { unreachable!() };
    *status = SubagentStatus::Cancelled;
    mutation(&mut state, &child, 1, &cancelled);
    assert_eq!(serde_json::to_value(&state).unwrap(), before);

    let (mut failed_state, child) = terminal_cancel_fixture();
    mutation(
        &mut failed_state, &child, 1,
        &OrchestrationAction::SubagentControlDone {
            seq: 1, applied: false, error: Some("native cancellation failed".into()),
        },
    );
    assert_eq!(failed_state.subagents[0].status, SubagentStatus::Running);
    assert_eq!(failed_state.subagents[0].last_control.as_ref().unwrap().stage, ControlStage::Failed);
    assert_eq!(failed_state.subagents[0].last_control.as_ref().unwrap().error.as_deref(), Some("native cancellation failed"));
    assert_error(&mut failed_state, &child, 1, &cancelled, "subagent_control_conflict");
}

#[test]
fn terminal_cancel_rejects_wrong_subject_epoch_session_sequence_or_intent() {
    let (state, child) = terminal_cancel_fixture();
    let Actor::Agent(child) = child else { unreachable!() };
    for case in [
        "subject", "epoch", "missing-session", "session", "sequence",
        "intent-sequence", "intent-op", "payload", "unregistered",
    ] {
        let mut state = state.clone();
        let mut caller = child.clone();
        let mut seq = 1;
        let expected = match case {
            "subject" => {
                caller.auth = fixture_auth("worker", Some("wrong-child"));
                "actor_forbidden"
            }
            "epoch" => { caller.auth.attachment_epoch = 2; "job_unauthorized" }
            "missing-session" => { caller.omp_session_id = None; "session_mismatch" }
            "session" => { caller.omp_session_id = Some("wrong-session".into()); "session_mismatch" }
            "sequence" => { seq = 2; "subagent_control_not_found" }
            "intent-sequence" => {
                state.subagents[0].last_control.as_mut().unwrap().seq = 2;
                "subagent_control_conflict"
            }
            "intent-op" => {
                state.subagents[0].last_control.as_mut().unwrap().op = SubagentOp::Send { text: "not cancel".into() };
                "subagent_control_conflict"
            }
            "payload" => {
                state.messages[0].text = serde_json::json!({
                    "subagent_id": "child", "op": { "op": "send", "text": "not cancel" },
                }).to_string();
                "session_mismatch"
            }
            _ => { state.subagents.clear(); "subagent_not_found" }
        };
        let before = serde_json::to_value(&state).unwrap();
        assert_error(
            &mut state, &Actor::Agent(caller), 1,
            &OrchestrationAction::SubagentControlDone { seq, applied: true, error: None },
            expected,
        );
        assert_eq!(serde_json::to_value(&state).unwrap(), before, "{case}");
    }
}

#[test]
fn child_telemetry_cannot_create_registration_or_claim_unrequested_cancellation() {
    let mut state = state();
    let child = subagent_actor("worker", "child");
    assert_error(&mut state, &child, 1, &update("child", None), "subagent_not_found");
    mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
    mutation(&mut state, &child, 1, &update("child", None));
    let mut cancelled = update("child", None);
    let OrchestrationAction::SubagentUpdate { status, .. } = &mut cancelled else { unreachable!() };
    *status = SubagentStatus::Cancelled;
    let before = serde_json::to_value(&state).unwrap();
    assert_error(&mut state, &child, 1, &cancelled, "subagent_control_conflict");
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}
