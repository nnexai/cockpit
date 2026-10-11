use std::{future::Future, sync::Arc, time::Duration};

use clap::Parser;
use cockpit_core::orchestration::{Actor, AgentCaller, OrchestrationService};
use cockpit_protocol::orchestration::{
    AttachmentLaunchMode, CloseReason, DeliveryStage, DispatchStep, MessageKind, NativeDeferReason,
    NativeProcessIdentity, NativeShellIdentity, OrchestrationAction, OrchestrationSnapshot,
    OrchestrationSnapshotRequest, RetirementIdentity, RetirementPhase, RetirementState, Run,
    RunLocation, RunStage,
};
use sha2::{Digest, Sha256};

use super::super::{
    CliError,
    args::InboxCommand,
    caller::native_process_evidence,
    context::Context,
    retirement::{
        completed_main_wait, empty_main_wait_read, main_wait_deadline_run, main_wait_read,
        retirement_wait, retirement_wait_ready, validate_retirement_token,
    },
    wait::{InboxCounts, KindCount, next_snapshot},
};
use super::{
    TestCli, TestCommand, accept_retirement, retirement_fixture, socket::SocketFixture,
    test_checkout_claim, test_credential_record, test_job_auth,
};

#[test]
fn retirement_wait_tracks_acceptance_and_never_spins_on_unchanged_offer() {
    let (mut run, caller) = retirement_fixture();
    assert!(!retirement_wait_ready(None, run.retirement.as_ref()));
    run.stage = RunStage::Reported;
    assert!(!retirement_wait_ready(None, run.retirement.as_ref()));
    accept_retirement(&mut run, &caller);
    assert!(retirement_wait_ready(None, run.retirement.as_ref()));
    let offered = run.retirement.clone().unwrap();
    assert!(!retirement_wait_ready(Some(&offered), Some(&offered)));
    let mut metadata_only = offered.clone();
    metadata_only.updated_at = "2026-10-07T00:00:02Z".into();
    assert!(retirement_wait_ready(Some(&offered), Some(&metadata_only)));
    let mut deferred = offered.clone();
    deferred.state = RetirementState::NativeStopDeferred {
        offered_at: "2026-10-07T00:00:01Z".into(),
        reason: NativeDeferReason::Busy,
        at: "2026-10-07T00:00:01Z".into(),
    };
    // Same timestamp does not hide a real state change.
    assert!(retirement_wait_ready(Some(&offered), Some(&deferred)));
    assert!(!retirement_wait_ready(Some(&deferred), Some(&deferred)));
    deferred.state = RetirementState::Unknown {
        at: "2026-10-07T00:00:02Z".into(),
        phase: RetirementPhase::NativeStop,
        detail: "stop not confirmed".into(),
    };
    assert!(retirement_wait_ready(Some(&deferred), Some(&deferred)));
}

#[test]
fn inbox_retirement_envelope_never_exposes_mail_after_acceptance() {
    let (mut run, caller) = retirement_fixture();
    let counts = || InboxCounts {
        run_id: run.run_id.clone(),
        pending: true,
        through_seq: 99,
        counts: vec![KindCount {
            kind: MessageKind::Instruction,
            count: 3,
        }],
    };
    let open = serde_json::to_value(main_wait_read(&run, Some(counts())).unwrap()).unwrap();
    assert_eq!(open["mode"], "open");
    validate_retirement_token(open["retirement_token"].as_str().unwrap()).unwrap();
    assert_eq!(open["inbox"]["through_seq"], 99);
    assert!(open["retirement"].is_null());
    let inbox = counts();
    accept_retirement(&mut run, &caller);
    let closed = serde_json::to_value(main_wait_read(&run, Some(inbox)).unwrap()).unwrap();
    assert_eq!(closed["mode"], "retirement_only");
    assert_eq!(closed.as_object().unwrap().len(), 3);
    validate_retirement_token(closed["retirement_token"].as_str().unwrap()).unwrap();
    assert!(closed.get("inbox").is_none());
    run.close_reason = Some(CloseReason::Cancelled);
    assert!(
        main_wait_read(
            &run,
            Some(InboxCounts {
                run_id: run.run_id.clone(),
                pending: false,
                through_seq: 0,
                counts: vec![],
            })
        )
        .is_err()
    );
    let parsed = TestCli::try_parse_from([
        "test",
        "inbox",
        "wait",
        "--with-retirement",
        "--omp-pid",
        "1234",
    ])
    .unwrap();
    let TestCommand::Inbox(args) = parsed.command else {
        panic!("expected inbox")
    };
    assert!(matches!(
        args.command,
        InboxCommand::Wait {
            with_retirement: true,
            ..
        }
    ));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn retirement_wait_tracks_accepted_metadata_without_quiet_herdr_reads() {
    let fixture = SocketFixture::new().await;
    let (context, mut run, caller) = main_wait_socket_context(&fixture).await;
    accept_retirement(&mut run, &caller);
    publish_main_wait_run(&fixture, &run);
    let (initial_ready, initial_response) = fixture.gate(0);
    let (mut final_ready, final_response) = fixture.gate(0);
    let waiting_context = context.clone();
    let mut waiting = tokio::spawn(async move { retirement_wait(&waiting_context, true, 10).await });
    main_wait_test_step("accepted wait initial physical read", &fixture, async {
        tokio::select! {
            result = initial_ready => result.unwrap(),
            result = &mut waiting => panic!("wait completed before initial physical read: {result:?}"),
        }
    })
    .await;
    initial_response
        .send(Some(fixture.payload.lock().clone()))
        .unwrap();
    // A second Herdr read during the quiet wait would hit the final gate.
    assert!(
        tokio::time::timeout(Duration::from_millis(1100), &mut final_ready)
            .await
            .is_err()
    );
    // Republishing an unchanged accepted offer must not make a quiet wait
    // perform another physical read or return unchanged metadata.
    publish_main_wait_run(&fixture, &run);
    assert!(
        tokio::time::timeout(Duration::from_millis(1100), &mut final_ready)
            .await
            .is_err()
    );
    run.retirement.as_mut().unwrap().state = RetirementState::NativeStopDeferred {
        offered_at: "2026-10-07T00:00:01Z".into(),
        reason: NativeDeferReason::Busy,
        at: "2026-10-07T00:00:02Z".into(),
    };
    publish_main_wait_run(&fixture, &run);
    let committed = fixture.bytes().unwrap();
    tokio::time::timeout(Duration::from_secs(3), final_ready)
        .await
        .unwrap()
        .unwrap();
    final_response
        .send(Some(fixture.payload.lock().clone()))
        .unwrap();
    let accepted = main_wait_test_step("accepted metadata wait completion", &fixture, waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(accepted.stage, RunStage::Closed);
    let retirement = accepted.retirement.unwrap();
    assert_eq!(retirement.retirement_id, "retirement");
    assert!(matches!(
        retirement.state,
        RetirementState::NativeStopDeferred { reason: NativeDeferReason::Busy, .. }
    ));
    assert_eq!(fixture.bytes().unwrap(), committed, "retirement observation cannot write");
}

#[cfg(target_os = "linux")]
async fn main_wait_socket_context(fixture: &SocketFixture) -> (Arc<Context>, Run, AgentCaller) {
    use std::os::unix::fs::MetadataExt;
    let root_id =
        main_wait_test_step("adopting socket-backed root", fixture, fixture.adopt()).await;

    let mut caller = fixture.physical_caller();
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let parent: u32 = stat[stat.rfind(')').unwrap() + 1..]
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    caller.process = Some(native_process_evidence(parent).unwrap());
    caller.provenance.as_mut().unwrap().process = caller.process.clone();
    let (mut run, _) = retirement_fixture();
    run.run_id = uuid::Uuid::new_v4().to_string();
    caller.env_run = Some((run.run_id.clone(), run.attempt));
    let task_id = uuid::Uuid::new_v4().to_string();
    fixture.operator(OrchestrationAction::TaskCreate {
        root_id: root_id.clone(),
        task_id: task_id.clone(),
        title: "Observe the exact worker incarnation".into(),
        description: "Retain durable mail and do not authorize another source writer.".into(),
        depends_on: vec![],
        follow_up_of: None,
        expected_doc_revision: None,
        source_revision: None,
    });
    let task_snapshot = fixture
        .context
        .service
        .snapshot_saved(&OrchestrationSnapshotRequest {
            session_id: caller.session_id.clone(),
            root_id: Some(root_id.clone()),
        })
        .unwrap();
    let task = &task_snapshot
        .board
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|view| view.task.task_id == task_id)
        .unwrap()
        .task;
    run.task_id = Some(task_id);
    run.task_revision_at_start = Some(task.task_revision.clone());
    run.attachment.as_mut().unwrap().process = caller.process.clone();
    run.root_id = root_id.clone();
    run.parent_run_id = Some(root_id);
    run.attachment.as_mut().unwrap().main_session_id = caller.omp_session_id.clone();
    run.attachment.as_mut().unwrap().location = Some(RunLocation {
        boot_id: caller.boot_id.clone(),
        terminal_id: caller.terminal_id.clone(),
        native_session_id: caller.native_session_id.clone(),
        endpoint_identity: caller.endpoint_identity.clone(),
        session_id: caller.session_id.clone(),
        workspace_id: caller.workspace_id.clone(),
        tab_id: caller.tab_id.clone(),
        pane_id: caller.pane_id.clone(),
        launch_tag: "launch".into(),
    });
    run.dispatch.as_mut().unwrap().endpoint_identity = Some(caller.endpoint_identity.clone());
    // Use a live CLI ancestor and its executable/argv evidence, not the parser
    // fixture's imaginary PID. Acceptance copies this exact launch baseline.
    let executable = std::fs::metadata(format!("/proc/{parent}/exe")).unwrap();
    let argv = std::fs::read(format!("/proc/{parent}/cmdline")).unwrap();
    run.attachment.as_mut().unwrap().shell = Some(NativeShellIdentity {
        process: caller.process.clone().unwrap(),
        executable_device: executable.dev().to_string(),
        executable_inode: executable.ino().to_string(),
        argv_digest: format!("{:x}", Sha256::digest(argv)),
    });
    // The observed executable is the native process itself, not a distinct
    // shell parent; keep the typed launch layout coherent with this evidence.
    run.attachment.as_mut().unwrap().launch_mode = Some(AttachmentLaunchMode::DirectProcess);
    let context = Arc::new(Context {
        service: OrchestrationService::open(&fixture.configuration).unwrap(),
        adapter: fixture.context.adapter.clone(),
        session: caller.session_id.clone(),
        actor: Some(Actor::Agent(caller.clone())),
        evidence: fixture.context.evidence.clone(),
        configuration: fixture.configuration.clone(),
        retirement: true,
        child_registration: false,
    });
    publish_main_wait_run(fixture, &run);
    (context, run, caller)
}

#[cfg(target_os = "linux")]
fn publish_main_wait_run(fixture: &SocketFixture, run: &Run) {
    // Model an atomic durable acceptance/rebinding by the owner, retaining
    // actual service-created mail and delivery/ACK state byte-for-byte.
    let mut state: serde_json::Value = serde_json::from_slice(&fixture.bytes().unwrap()).unwrap();
    state["revision"] = serde_json::json!(state["revision"].as_u64().unwrap() + 1);
    let runs = state["runs"].as_array_mut().unwrap();
    let serialized = serde_json::to_value(run).unwrap();
    if let Some(existing) = runs
        .iter_mut()
        .find(|existing| existing["run_id"] == run.run_id)
    {
        *existing = serialized;
    } else {
        runs.push(serialized);
    }
    let credentials = state["credentials"].as_array_mut().unwrap();
    let auth = test_job_auth();
    if !credentials.iter().any(|record| record["credential_id"] == auth.credential_id) {
        credentials.push(test_credential_record(run, &auth, "worker"));
    }
    let claims = state["checkout_claims"].as_array_mut().unwrap();
    if !claims.iter().any(|claim| claim["claim_id"] == run.checkout_claim_id.as_deref().unwrap()) {
        claims.push(test_checkout_claim(run));
    }
    let temporary = fixture.context.service.base().join("main-wait-test.next");
    std::fs::write(&temporary, serde_json::to_vec(&state).unwrap()).unwrap();
    std::fs::rename(temporary, fixture.context.service.base().join("state.json")).unwrap();
}

#[cfg(target_os = "linux")]
async fn main_wait_test_step<T>(
    phase: &str,
    fixture: &SocketFixture,
    future: impl Future<Output = T>,
) -> T {
    tokio::pin!(future);
    // A Tokio timeout cannot guard a forever-runnable barrier while time is
    // paused. Bound scheduler turns instead; exhaustion FAILS with the phase
    // and socket queue, never fabricates a successful consumer result.
    for _ in 0..16_384 {
        let result = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(match future.as_mut().poll(cx) {
                std::task::Poll::Ready(value) => Some(value),
                std::task::Poll::Pending => None,
            })
        })
        .await;
        if let Some(value) = result {
            return value;
        }
        tokio::task::yield_now().await;
    }
    panic!(
        "S7 watchdog exhausted scheduler turns: phase={phase}, queued_reads={}, server_finished={}, clock={:?}",
        fixture.gates.lock().len(),
        fixture.server.is_finished(),
        tokio::time::Instant::now(),
    );
}

#[cfg(target_os = "linux")]
async fn main_wait_timeout_fence(
    fixture: &SocketFixture,
    context: Arc<Context>,
    initial: OrchestrationSnapshot,
    run: &Run,
    case: &str,
) -> (
    tokio::task::JoinHandle<Result<Option<OrchestrationSnapshot>, CliError>>,
    tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
) {
    // A durable revision wakes wait_next immediately. Hold its snapshot
    // precheck so next_snapshot must take the actual timeout branch.
    publish_main_wait_run(fixture, run);
    let (blocked_ready, blocked_response) = fixture.gate(0);
    let (fence_ready, fence_response) = fixture.gate(0);
    tokio::time::pause();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    let mut waiting = tokio::spawn(async move { next_snapshot(&context, &initial, deadline).await });
    main_wait_test_step(
        &format!("{case}: snapshot precheck gate"),
        fixture,
        async {
            tokio::select! {
                result = blocked_ready => result.unwrap(),
                result = &mut waiting => panic!("{case}: wait completed before snapshot gate: {result:?}"),
            }
        },
    )
    .await;
    // Advance PAST the deadline, including the timer wheel's next tick.
    // Advancing to equality and then spinning a runnable barrier prevents
    // paused-time auto-advance and can strand the timeout at its last tick.
    tokio::time::advance(Duration::from_millis(101)).await;
    assert!(tokio::time::Instant::now() > deadline);
    main_wait_test_step(
        &format!("{case}: post-timeout authority gate"),
        fixture,
        async {
            tokio::select! {
                result = fence_ready => result.unwrap(),
                result = &mut waiting => panic!("{case}: wait completed before deadline fence: {result:?}"),
            }
        },
    )
    .await;
    tokio::time::resume();
    assert!(
        !waiting.is_finished(),
        "fresh final authority fence must outlive deadline"
    );
    drop(blocked_response);
    (waiting, fence_response)
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn main_wait_deadline_reconstructs_accepted_metadata_without_mail_or_writes() {
    let fixture = SocketFixture::new().await;
    let (context, mut run, caller) = main_wait_test_step(
        "initializing accepted-mail context",
        &fixture,
        main_wait_socket_context(&fixture),
    )
    .await;
    let secret = "SECRET instruction body: must never reach accepted main waiter";
    fixture.send(&run.run_id, secret);
    let initial = main_wait_test_step("open snapshot", &fixture, context.snapshot(None))
        .await
        .unwrap();
    assert_eq!(
        initial
            .messages
            .iter()
            .find(|m| m.text == secret)
            .unwrap()
            .stage,
        DeliveryStage::Stored
    );
    let after = initial.messages.iter().map(|m| m.seq).max().unwrap();
    let open = completed_main_wait(&context, &initial, after).unwrap();
    let open_json = serde_json::to_value(&open).unwrap();
    assert_eq!(open_json["mode"], "open");
    assert!(!open.ready(open_json["retirement_token"].as_str()));
    drop(open);
    let (waiting, final_response) =
        main_wait_timeout_fence(&fixture, context.clone(), initial, &run, "acceptance").await;
    // The budget is already exhausted while Open; acceptance happens while
    // its fresh authority fence is blocked, before any final output.
    let late_secret = "SECRET late durable body: no counts or ACK after acceptance";
    fixture.send(&run.run_id, late_secret);
    accept_retirement(&mut run, &caller);
    
    publish_main_wait_run(&fixture, &run);
    let committed = fixture.bytes().unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&committed).unwrap();
    assert!(
        stored["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == secret)
    );
    assert!(
        stored["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == late_secret)
    );
    final_response
        .send(Some(fixture.payload.lock().clone()))
        .unwrap();
    assert!(
        main_wait_test_step("accepted deadline completion", &fixture, waiting)
            .await
            .unwrap()
            .unwrap()
            .is_none(),
        "must exercise deadline reconstruction"
    );
    // These are the production deadline-output consumer calls, not a
    // hand-built envelope or a replay of the previous Open snapshot.
    let current = main_wait_deadline_run(&context).unwrap();
    let deadline_output =
        serde_json::to_value(empty_main_wait_read(&current, after).unwrap()).unwrap();
    assert_eq!(deadline_output["mode"], "retirement_only");
    assert_eq!(deadline_output.as_object().unwrap().len(), 3);
    assert_eq!(deadline_output["retirement"]["retirement_id"], "retirement");
    validate_retirement_token(deadline_output["retirement_token"].as_str().unwrap()).unwrap();
    assert!(deadline_output.get("inbox").is_none());
    assert!(deadline_output.get("counts").is_none());
    assert!(deadline_output.get("through_seq").is_none());
    let encoded = serde_json::to_string(&deadline_output).unwrap();
    assert!(!encoded.contains(secret));
    assert!(!encoded.contains(late_secret));
    assert_eq!(
        fixture.bytes().unwrap(),
        committed,
        "deadline read must not ACK or write"
    );

    // Also exercise the completed-snapshot consumer with genuine secret
    // messages still present, rather than giving it an empty mail vector.
    let accepted =
        main_wait_test_step("accepted secret snapshot", &fixture, context.snapshot(None))
            .await
            .unwrap();
    for body in [secret, late_secret] {
        assert_eq!(
            accepted
                .messages
                .iter()
                .find(|m| m.text == body)
                .unwrap()
                .stage,
            DeliveryStage::Stored
        );
    }
    let completed =
        serde_json::to_value(completed_main_wait(&context, &accepted, after).unwrap()).unwrap();
    assert_eq!(completed, deadline_output);
    assert_eq!(
        fixture.bytes().unwrap(),
        committed,
        "completed read must not ACK or write"
    );
}

#[cfg(target_os = "linux")]
fn main_wait_retirement_identity(run: &mut Run) -> &mut RetirementIdentity {
    run.retirement.as_mut().unwrap().identity.as_mut().unwrap()
}

#[cfg(target_os = "linux")]
fn main_wait_durable_replacements() -> &'static [(&'static str, &'static str, fn(&mut Run))] {
    &[
        ("run attempt", "attempt_stale", |r| r.attempt += 1),
        ("attachment launch attempt", "caller_mismatch", |r| {
            r.attachment.as_mut().unwrap().launch_attempt += 1
        }),
        ("bound native session", "session_mismatch", |r| {
            r.attachment.as_mut().unwrap().main_session_id = Some("replacement".into())
        }),
        ("bound PID", "caller_mismatch", |r| {
            r.attachment.as_mut().unwrap().process.as_mut().unwrap().pid += 1
        }),
        ("bound PID incarnation", "caller_mismatch", |r| {
            r.attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1
        }),
        ("bound kernel boot", "caller_mismatch", |r| {
            r.attachment.as_mut().unwrap().process.as_mut().unwrap().kernel_boot_id = Some("replacement".into())
        }),
        ("dispatch launch attempt", "caller_mismatch", |r| {
            r.dispatch.as_mut().unwrap().launch_attempt += 1
        }),
        ("dispatch launch tag", "caller_mismatch", |r| {
            r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into())
        }),
        ("dispatch endpoint", "caller_mismatch", |r| {
            r.dispatch.as_mut().unwrap().endpoint_identity = Some("replacement".into())
        }),
        ("current launch shell", "caller_mismatch", |r| {
            r.attachment.as_mut().unwrap().shell.as_mut().unwrap().argv_digest = "b".repeat(64)
        }),
        ("location native session", "caller_mismatch", |r| {
            r.attachment.as_mut().unwrap().location.as_mut().unwrap().native_session_id = Some("replacement".into())
        }),
        ("location launch tag", "caller_mismatch", |r| {
            r.attachment.as_mut().unwrap().location.as_mut().unwrap().launch_tag = "replacement".into()
        }),
        ("retirement attachment ID", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).attachment_id = Some("replacement".into())
        }),
        ("retirement attachment epoch", "caller_mismatch", |r| {
            let identity = main_wait_retirement_identity(r);
            identity.attachment_epoch = Some(identity.attachment_epoch.unwrap() + 1)
        }),
        ("missing retirement attachment ID", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).attachment_id = None
        }),
        ("missing retirement attachment epoch", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).attachment_epoch = None
        }),
        ("retirement run attempt", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).run_attempt += 1
        }),
        ("retirement launch attempt", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).launch_attempt += 1
        }),
        ("retirement launch tag", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).launch_tag = "replacement".into()
        }),
        ("retirement endpoint", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).endpoint_identity = "replacement".into()
        }),
        ("retirement session", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).session_id = "replacement".into()
        }),
        ("retirement workspace", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).workspace_id = "replacement".into()
        }),
        ("retirement tab", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).tab_id = "replacement".into()
        }),
        ("retirement pane", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).pane_id = "replacement".into()
        }),
        ("retirement terminal", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).terminal_id = "replacement".into()
        }),
        ("retirement Herdr boot", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).herdr_boot_id = Some("replacement".into())
        }),
        ("retirement native session", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).omp_session_id = "replacement".into()
        }),
        ("retirement PID", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).process.pid += 1
        }),
        ("retirement PID incarnation", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).process.start_ticks += 1
        }),
        ("retirement kernel boot", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).process.kernel_boot_id = Some("replacement".into())
        }),
        ("retirement shell PID", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).shell.process.pid += 1
        }),
        ("retirement shell incarnation", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).shell.process.start_ticks += 1
        }),
        ("retirement shell kernel boot", "caller_mismatch", |r| {
            main_wait_retirement_identity(r)
                .shell
                .process
                .kernel_boot_id = Some("replacement".into())
        }),
        (
            "retirement shell executable device",
            "caller_mismatch",
            |r| main_wait_retirement_identity(r).shell.executable_device = "replacement".into(),
        ),
        (
            "retirement shell executable inode",
            "caller_mismatch",
            |r| main_wait_retirement_identity(r).shell.executable_inode = "replacement".into(),
        ),
        ("retirement shell argv", "caller_mismatch", |r| {
            main_wait_retirement_identity(r).shell.argv_digest = "b".repeat(64)
        }),
        ("missing immutable identity", "caller_mismatch", |r| {
            r.retirement.as_mut().unwrap().identity = None
        }),
    ]
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn main_wait_consumers_reject_durable_incarnation_replacement_at_deadline() {
    let fixture = SocketFixture::new().await;
    let (context, mut baseline, caller) = main_wait_test_step(
        "initializing durable-replacement context",
        &fixture,
        main_wait_socket_context(&fixture),
    )
    .await;
    fixture.send(
        &baseline.run_id,
        "SECRET mail retained during identity rejection",
    );
    accept_retirement(&mut baseline, &caller);
    
    let replacements = main_wait_durable_replacements();
    for (name, code, replace) in replacements {
        publish_main_wait_run(&fixture, &baseline);
        let initial = main_wait_test_step(
            &format!("{name}: baseline snapshot"),
            &fixture,
            context.snapshot(None),
        )
        .await
        .unwrap();
        assert!(
            completed_main_wait(&context, &initial, 0).is_ok(),
            "{name}: valid baseline"
        );
        let (waiting, final_response) =
            main_wait_timeout_fence(&fixture, context.clone(), initial, &baseline, name).await;
        let mut replaced = baseline.clone();
        replace(&mut replaced);
        publish_main_wait_run(&fixture, &replaced);
        let committed = fixture.bytes().unwrap();
        final_response
            .send(Some(fixture.payload.lock().clone()))
            .unwrap();
        assert!(
            main_wait_test_step(&format!("{name}: deadline completion"), &fixture, waiting)
                .await
                .unwrap()
                .unwrap()
                .is_none(),
            "{name}: actual deadline"
        );
        let error = main_wait_deadline_run(&context).unwrap_err();
        assert_eq!(error.code, *code, "{name}: deadline consumer");
        // The receipt command performs this same durable scope preflight
        // before calling mutate; no receipt is submitted on rejection.
        assert_eq!(
            context.retiring_run_for_review().unwrap_err().code,
            *code,
            "{name}: receipt preflight"
        );
        let snapshot = main_wait_test_step(
            &format!("{name}: replaced snapshot"),
            &fixture,
            context.snapshot(None),
        )
        .await
        .unwrap();
        let error = completed_main_wait(&context, &snapshot, 0)
            .err()
            .expect("replacement must reject metadata");
        assert_eq!(error.code, *code, "{name}: completed consumer");
        assert_eq!(
            fixture.bytes().unwrap(),
            committed,
            "{name}: no read/receipt/ACK write"
        );
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn main_wait_deadline_rejects_runtime_and_live_process_replacement_without_writes() {
    let fixture = SocketFixture::new().await;
    let (context, mut run, caller) = main_wait_test_step(
        "initializing runtime-replacement context",
        &fixture,
        main_wait_socket_context(&fixture),
    )
    .await;
    fixture.send(&run.run_id, "SECRET mail retained at final runtime fence");
    accept_retirement(&mut run, &caller);
    
    let runtime_replacements: &[(&str, fn(&mut serde_json::Value))] = &[
        ("native session", |v| {
            v["snapshot"]["agents"][0]["agent_session"]["value"] = serde_json::json!("replacement")
        }),
        ("terminal", |v| {
            v["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement")
        }),
        ("Herdr boot", |v| {
            v["snapshot"]["boot_id"] = serde_json::json!("replacement")
        }),
        ("native agent kind", |v| {
            v["snapshot"]["agents"][0]["agent"] = serde_json::json!("other")
        }),
        ("launch pending", |v| {
            v["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true)
        }),
    ];
    for (name, replace) in runtime_replacements {
        publish_main_wait_run(&fixture, &run);
        let initial = main_wait_test_step(
            &format!("{name}: runtime baseline snapshot"),
            &fixture,
            context.snapshot(None),
        )
        .await
        .unwrap();
        assert!(
            completed_main_wait(&context, &initial, 0).is_ok(),
            "{name}: valid baseline"
        );
        let (waiting, final_response) =
            main_wait_timeout_fence(&fixture, context.clone(), initial, &run, name).await;
        let committed = fixture.bytes().unwrap();
        let mut payload = fixture.payload.lock().clone();
        replace(&mut payload);
        final_response.send(Some(payload)).unwrap();
        assert_eq!(
            main_wait_test_step(
                &format!("{name}: rejected final runtime fence"),
                &fixture,
                waiting
            )
            .await
            .unwrap()
            .unwrap_err()
            .code,
            "caller_mismatch",
            "{name}"
        );
        assert_eq!(
            fixture.bytes().unwrap(),
            committed,
            "{name}: timeout fence cannot write"
        );
    }
    check_main_wait_live_process_replacements(&fixture, &context, &run, &caller).await;
}

#[cfg(target_os = "linux")]
async fn check_main_wait_live_process_replacements(
    fixture: &SocketFixture,
    context: &Arc<Context>,
    run: &Run,
    caller: &AgentCaller,
) {
    let process_replacements: &[(&str, fn(&mut NativeProcessIdentity))] = &[
        ("PID", |p| p.pid = u32::MAX),
        ("PID incarnation", |p| p.start_ticks += 1),
        ("kernel boot", |p| {
            p.kernel_boot_id = Some("replacement".into())
        }),
    ];
    for (name, replace) in process_replacements {
        publish_main_wait_run(fixture, run);
        let initial = main_wait_test_step(
            &format!("{name}: process baseline snapshot"),
            fixture,
            context.snapshot(None),
        )
        .await
        .unwrap();
        let completed_snapshot = initial.clone();
        let (waiting, final_response) =
            main_wait_timeout_fence(fixture, context.clone(), initial, run, name).await;
        final_response
            .send(Some(fixture.payload.lock().clone()))
            .unwrap();
        assert!(
            main_wait_test_step(
                &format!("{name}: process deadline completion"),
                fixture,
                waiting
            )
            .await
            .unwrap()
            .unwrap()
            .is_none(),
            "{name}: actual deadline"
        );
        let mut replaced_caller = caller.clone();
        replace(replaced_caller.process.as_mut().unwrap());
        replaced_caller.provenance.as_mut().unwrap().process = replaced_caller.process.clone();
        let mut replaced_run = run.clone();
        // Keep durable authority internally coherent with the replacement.
        // Only live ancestor evidence can reject this fabricated incarnation.
        replaced_run.attachment.as_mut().unwrap().process = replaced_caller.process.clone();
        replaced_run.attachment.as_mut().unwrap().shell.as_mut().unwrap().process =
            replaced_caller.process.clone().unwrap();
        replaced_run
            .retirement
            .as_mut()
            .unwrap()
            .identity
            .as_mut()
            .unwrap()
            .process = replaced_caller.process.clone().unwrap();
        main_wait_retirement_identity(&mut replaced_run).shell.process =
            replaced_caller.process.clone().unwrap();
        publish_main_wait_run(fixture, &replaced_run);
        let committed = fixture.bytes().unwrap();
        let replaced_context = Context {
            service: OrchestrationService::open(&fixture.configuration).unwrap(),
            adapter: context.adapter.clone(),
            session: context.session.clone(),
            actor: Some(Actor::Agent(replaced_caller)),
            evidence: context.evidence.clone(),
            configuration: fixture.configuration.clone(),
            retirement: true,
            child_registration: false,
        };
        assert!(
            replaced_context.retiring_run_for_review().is_ok(),
            "{name}: durable scope alone is insufficient"
        );
        assert_eq!(
            main_wait_deadline_run(&replaced_context).unwrap_err().code,
            "caller_mismatch",
            "{name}: live process fence"
        );
        let snapshot_error = main_wait_test_step(
            &format!("{name}: replaced process snapshot"),
            fixture,
            replaced_context.snapshot(None),
        )
        .await
        .unwrap_err();
        assert_eq!(snapshot_error.code, "caller_mismatch", "{name}: snapshot live process fence");
        assert_eq!(
            completed_main_wait(&replaced_context, &completed_snapshot, 0)
                .err()
                .unwrap()
                .code,
            "caller_mismatch",
            "{name}: completed live process fence"
        );
        assert_eq!(
            fixture.bytes().unwrap(),
            committed,
            "{name}: rejected incarnation cannot write"
        );
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn main_wait_startup_not_ready_then_attested_observes_late_brief_without_writes() {
    let fixture = SocketFixture::new().await;
    let (baseline, mut run, mut caller) = main_wait_socket_context(&fixture).await;
    run.stage = RunStage::Starting;
    run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending;
    run.dispatch.as_mut().unwrap().agent_started = false;
    publish_main_wait_run(&fixture, &run);
    fixture.payload.lock()["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true);
    caller.actual_agent_kind = None;
    let context = Context {
        service: OrchestrationService::open(&fixture.configuration).unwrap(),
        adapter: baseline.adapter.clone(),
        session: baseline.session.clone(),
        actor: Some(Actor::Agent(caller.clone())),
        evidence: baseline.evidence.clone(),
        configuration: fixture.configuration.clone(),
        retirement: true,
        child_registration: false,
    };
    let committed = fixture.bytes().unwrap();
    let snapshot = context.snapshot(None).await.unwrap();
    assert_eq!(
        completed_main_wait(&context, &snapshot, 0)
            .err()
            .unwrap()
            .code,
        "caller_not_ready"
    );
    assert_eq!(
        main_wait_deadline_run(&context).unwrap_err().code,
        "caller_not_ready"
    );
    assert_eq!(fixture.bytes().unwrap(), committed);
    let replacements: &[(&str, &str, fn(&mut Run))] = &[
        ("session_mismatch", "main session", |r| {
            r.attachment.as_mut().unwrap().main_session_id = Some("replacement".into())
        }),
        ("caller_mismatch", "process incarnation", |r| {
            r.attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1
        }),
        ("caller_mismatch", "endpoint", |r| {
            r.attachment.as_mut().unwrap().location.as_mut().unwrap().endpoint_identity = "replacement".into()
        }),
        ("caller_mismatch", "pane", |r| {
            r.attachment.as_mut().unwrap().location.as_mut().unwrap().pane_id = "replacement".into()
        }),
        ("caller_mismatch", "dispatch tag", |r| {
            r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into())
        }),
        ("attempt_stale", "run attempt", |r| r.attempt += 1),
    ];
    for (code, name, replace) in replacements {
        let mut replaced = run.clone();
        replace(&mut replaced);
        publish_main_wait_run(&fixture, &replaced);
        let committed = fixture.bytes().unwrap();
        let snapshot = context.snapshot(None).await.unwrap();
        assert_eq!(
            completed_main_wait(&context, &snapshot, 0)
                .err()
                .unwrap()
                .code,
            *code,
            "{name}"
        );
        assert_eq!(
            main_wait_deadline_run(&context).unwrap_err().code,
            *code,
            "{name}: deadline"
        );
        assert_eq!(fixture.bytes().unwrap(), committed);
    }
    publish_main_wait_run(&fixture, &run);

    // Launch proof and the durable brief come later. The old opening
    // snapshot must ask for fresh evidence, not permanently revoke itself.
    fixture.payload.lock()["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(false);
    run.stage = RunStage::Working;
    run.dispatch.as_mut().unwrap().step = DispatchStep::Launched;
    run.dispatch.as_mut().unwrap().agent_started = true;
    publish_main_wait_run(&fixture, &run);
    fixture.send(&run.run_id, "late startup brief");
    let committed = fixture.bytes().unwrap();
    assert_eq!(
        context.check_caller().await.unwrap_err().code,
        "caller_not_ready"
    );
    assert_eq!(fixture.bytes().unwrap(), committed);

    caller.actual_agent_kind = Some("omp".into());
    let fresh = Context {
        service: OrchestrationService::open(&fixture.configuration).unwrap(),
        adapter: baseline.adapter.clone(),
        session: baseline.session.clone(),
        actor: Some(Actor::Agent(caller.clone())),
        evidence: baseline.evidence.clone(),
        configuration: fixture.configuration.clone(),
        retirement: true,
        child_registration: false,
    };
    let snapshot = fresh.snapshot(None).await.unwrap();
    let read = completed_main_wait(&fresh, &snapshot, 0).unwrap();
    let output = serde_json::to_value(&read).unwrap();
    assert_eq!(output["mode"], "open");
    assert_eq!(output["inbox"]["pending"], true);
    assert_eq!(output["inbox"]["counts"][0]["count"], 1);
    assert!(output["retirement"].is_null());
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("late startup brief")
    );
    assert!(main_wait_deadline_run(&fresh).is_ok());
    assert_eq!(
        fixture.bytes().unwrap(),
        committed,
        "observation never ACKs or pulls mail"
    );

    // Missing attestation on the now-mature run is not another startup.
    fixture.payload.lock()["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true);
    assert_eq!(
        context.retiring_run_for_review().unwrap_err().code,
        "caller_mismatch"
    );
    assert_eq!(
        fresh.check_caller().await.unwrap_err().code,
        "caller_mismatch"
    );
    assert_eq!(fixture.bytes().unwrap(), committed);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn main_wait_startup_settling_fences_current_and_deadline_observation() {
    let fixture = SocketFixture::new().await;
    let (baseline, mut run, mut caller) = main_wait_socket_context(&fixture).await;
    run.stage = RunStage::Starting;
    run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending;
    run.dispatch.as_mut().unwrap().agent_started = false;
    caller.actual_agent_kind = None;
    caller.native_session_id = None;
    let context = Arc::new(Context {
        service: OrchestrationService::open(&fixture.configuration).unwrap(),
        adapter: baseline.adapter.clone(),
        session: baseline.session.clone(),
        actor: Some(Actor::Agent(caller.clone())),
        evidence: baseline.evidence.clone(),
        configuration: fixture.configuration.clone(),
        retirement: true,
        child_registration: false,
    });
    let attested = fixture.payload.lock().clone();
    let mut pending = attested.clone();
    pending["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true);
    pending["snapshot"]["agents"][0]
        .as_object_mut()
        .unwrap()
        .remove("agent_session");
    let replacements: &[(&str, &str, fn(&mut serde_json::Value), fn(&mut Run))] = &[
        (
            "expected native session",
            "caller_not_ready",
            |_| {},
            |_| {},
        ),
        (
            "foreign native session",
            "caller_mismatch",
            |v| {
                v["snapshot"]["agents"][0]["agent_session"]["value"] =
                    serde_json::json!("replacement")
            },
            |_| {},
        ),
        (
            "foreign boot",
            "caller_mismatch",
            |v| v["snapshot"]["boot_id"] = serde_json::json!("replacement"),
            |_| {},
        ),
        (
            "foreign terminal",
            "caller_mismatch",
            |v| v["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement"),
            |_| {},
        ),
        (
            "foreign process incarnation",
            "caller_mismatch",
            |_| {},
            |r| r.attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1,
        ),
        (
            "foreign dispatch tag",
            "caller_mismatch",
            |_| {},
            |r| r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into()),
        ),
        (
            "foreign attempt",
            "caller_mismatch",
            |_| {},
            |r| r.attempt += 1,
        ),
    ];
    for (name, code, replace_runtime, replace_run) in replacements {
        *fixture.payload.lock() = pending.clone();
        publish_main_wait_run(&fixture, &run);
        let initial = context.snapshot(None).await.unwrap();
        assert_eq!(
            completed_main_wait(&context, &initial, 0)
                .err()
                .unwrap()
                .code,
            "caller_not_ready"
        );
        let (waiting, response) =
            main_wait_timeout_fence(&fixture, context.clone(), initial, &run, name).await;
        let mut replaced_run = run.clone();
        replace_run(&mut replaced_run);
        publish_main_wait_run(&fixture, &replaced_run);
        let committed = fixture.bytes().unwrap();
        let mut changed = attested.clone();
        replace_runtime(&mut changed);
        response.send(Some(changed.clone())).unwrap();
        assert_eq!(
            main_wait_test_step(name, &fixture, waiting)
                .await
                .unwrap()
                .unwrap_err()
                .code,
            *code
        );
        *fixture.payload.lock() = changed;
        assert_eq!(
            context.check_caller().await.unwrap_err().code,
            *code,
            "{name}: current fence"
        );
        assert_eq!(
            fixture.bytes().unwrap(),
            committed,
            "{name}: no observation writes"
        );
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn main_wait_credentials_reject_replaced_attachment_before_physical_reads_or_writes() {
    let fixture = SocketFixture::new().await;
    let (context, mut baseline, caller) = main_wait_socket_context(&fixture).await;
    accept_retirement(&mut baseline, &caller);
    
    publish_main_wait_run(&fixture, &baseline);
    let snapshot = context.snapshot(None).await.unwrap();
    let replacements: &[fn(&mut Run)] = &[
        |run| run.attachment.as_mut().unwrap().attachment_id =
            "00000000-0000-4000-8000-000000000099".into(),
        |run| run.attachment.as_mut().unwrap().epoch += 1,
    ];
    // An invalid credential must fail before even opening the queued
    // physical authority read, not merely fail after a harmless RPC.
    let (mut physical_read, physical_response) = fixture.gate(0);
    for replace in replacements {
        let mut replaced = baseline.clone();
        replace(&mut replaced);
        publish_main_wait_run(&fixture, &replaced);
        let committed = fixture.bytes();
        assert_eq!(context.snapshot(None).await.unwrap_err().code, "job_unauthorized");
        assert_eq!(context.retiring_run_for_review().unwrap_err().code, "job_unauthorized");
        assert_eq!(main_wait_deadline_run(&context).unwrap_err().code, "job_unauthorized");
        assert_eq!(
            completed_main_wait(&context, &snapshot, 0).err().unwrap().code,
            "job_unauthorized",
            "a previously completed snapshot cannot authorize a replaced attachment",
        );
        assert_eq!(fixture.bytes(), committed, "stale attachment cannot write or ACK");
    }
    assert!(matches!(
        physical_read.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(fixture.gates.lock().len(), 1);
    drop(physical_response);
}
