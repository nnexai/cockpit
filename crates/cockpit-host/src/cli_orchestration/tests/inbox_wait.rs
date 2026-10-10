use std::{future::Future, time::Duration};

use cockpit_core::orchestration::{Actor, OrchestrationService};
use cockpit_protocol::orchestration::{
    DeliveryStage, MessageKind, OperatorOrigin, OrchestrationAction, OrchestrationMutationRequest,
    ReportKind, ReportOutcome, SubagentOp, SubagentStatus,
};

use super::super::wait::{ControlPayload, inbox_counts, next_snapshot, pending_controls};
use super::{
    message,
    socket::{EndpointChild, SocketFixture, reach_gate_without_advancing_time, socket_payload},
};

#[test]
fn wake_envelope_contains_only_counts_not_bodies_or_other_runs() {
    let messages = vec![
        message(
            "own",
            1,
            MessageKind::Instruction,
            DeliveryStage::Stored,
            "BELOW CURSOR",
        ),
        message(
            "own",
            2,
            MessageKind::Report,
            DeliveryStage::Read,
            "PRIVATE REPORT",
        ),
        message(
            "own",
            3,
            MessageKind::Report,
            DeliveryStage::Woken,
            "PRIVATE REPORT 2",
        ),
        message(
            "own",
            4,
            MessageKind::Instruction,
            DeliveryStage::Acked,
            "ACKED",
        ),
        message(
            "other",
            9,
            MessageKind::Instruction,
            DeliveryStage::Stored,
            "OTHER RUN",
        ),
    ];
    let result = inbox_counts(&messages, "own", 1);
    assert!(result.pending);
    assert_eq!(result.through_seq, 3);
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(
        value["counts"],
        serde_json::json!([{"kind": "report", "count": 2}])
    );
    assert_eq!(value.as_object().unwrap().len(), 4);
    assert!(!value.to_string().contains("PRIVATE"));
    assert!(!inbox_counts(&messages, "own", 3).pending);
    assert_eq!(messages[1].stage, DeliveryStage::Read);
}

#[test]
fn newer_subagent_control_never_advances_ordinary_wake_cursor() {
    let control = r#"{"subagent_id":"child","op":{"op":"send","text":"instruction"}}"#;
    let messages = vec![
        message(
            "own",
            2,
            MessageKind::Instruction,
            DeliveryStage::Stored,
            "ordinary",
        ),
        message(
            "own",
            9,
            MessageKind::SubagentControl,
            DeliveryStage::Stored,
            control,
        ),
    ];
    let ordinary = inbox_counts(&messages, "own", 1);
    assert!(ordinary.pending);
    assert_eq!(ordinary.through_seq, 2);
    assert_eq!(
        serde_json::to_value(ordinary).unwrap()["counts"],
        serde_json::json!([{"kind": "instruction", "count": 1}])
    );
    let after_ordinary = inbox_counts(&messages, "own", 2);
    assert!(!after_ordinary.pending);
    assert_eq!(after_ordinary.through_seq, 2);
    assert!(after_ordinary.counts.is_empty());
    let controls = pending_controls(&messages, "own", "child").unwrap();
    assert_eq!(
        controls
            .messages
            .iter()
            .map(|item| item.seq)
            .collect::<Vec<_>>(),
        vec![9]
    );
    assert_eq!(messages[1].stage, DeliveryStage::Stored);
}

#[test]
fn controls_are_read_only_and_scoped_to_own_run_and_subagent() {
    let own = r#"{"subagent_id":"child","op":{"op":"cancel"}}"#;
    let other = r#"{"subagent_id":"other","op":{"op":"send","text":"message"}}"#;
    let messages = vec![
        message(
            "own",
            4,
            MessageKind::SubagentControl,
            DeliveryStage::Read,
            own,
        ),
        message(
            "own",
            2,
            MessageKind::SubagentControl,
            DeliveryStage::Stored,
            own,
        ),
        message(
            "own",
            3,
            MessageKind::SubagentControl,
            DeliveryStage::Stored,
            other,
        ),
        message(
            "other",
            5,
            MessageKind::SubagentControl,
            DeliveryStage::Stored,
            own,
        ),
        message(
            "own",
            6,
            MessageKind::SubagentControl,
            DeliveryStage::Acked,
            own,
        ),
        message(
            "own",
            7,
            MessageKind::Instruction,
            DeliveryStage::Stored,
            "not JSON",
        ),
    ];
    let result = pending_controls(&messages, "own", "child").unwrap();
    assert_eq!(
        result
            .messages
            .iter()
            .map(|item| item.seq)
            .collect::<Vec<_>>(),
        vec![2, 4]
    );
    assert_eq!(
        serde_json::to_value(result)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(messages[0].stage, DeliveryStage::Read);
    assert_eq!(messages[1].stage, DeliveryStage::Stored);
}

#[tokio::test]
async fn caller_identity_and_membership_fail_before_any_durable_mutation() {
    let fixture = SocketFixture::new().await;
    let original = fixture.payload.lock().clone();
    let changes: &[(&str, fn(&mut serde_json::Value))] = &[
        ("absent pane", |v| {
            v["snapshot"]["panes"] = serde_json::json!([])
        }),
        ("absent tab", |v| {
            v["snapshot"]["tabs"] = serde_json::json!([])
        }),
        ("absent workspace", |v| {
            v["snapshot"]["workspaces"] = serde_json::json!([])
        }),
        ("wrong workspace", |v| {
            v["snapshot"]["panes"][0]["workspace_id"] = serde_json::json!("foreign")
        }),
        ("duplicate pane", |v| {
            let item = v["snapshot"]["panes"][0].clone();
            v["snapshot"]["panes"].as_array_mut().unwrap().push(item);
        }),
        ("duplicate tab", |v| {
            let item = v["snapshot"]["tabs"][0].clone();
            v["snapshot"]["tabs"].as_array_mut().unwrap().push(item);
        }),
        ("duplicate workspace", |v| {
            let item = v["snapshot"]["workspaces"][0].clone();
            v["snapshot"]["workspaces"]
                .as_array_mut()
                .unwrap()
                .push(item);
        }),
        ("agent membership", |v| {
            v["snapshot"]["agents"][0]["tab_id"] = serde_json::json!("foreign")
        }),
        ("terminal", |v| {
            v["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement")
        }),
        ("missing terminal", |v| {
            v["snapshot"]["panes"][0]
                .as_object_mut()
                .unwrap()
                .remove("terminal_id");
        }),
        ("boot", |v| {
            v["snapshot"]["boot_id"] = serde_json::json!("replacement")
        }),
        ("native session", |v| {
            v["snapshot"]["agents"][0]["agent_session"]["value"] = serde_json::json!("replacement")
        }),
        ("kind", |v| {
            v["snapshot"]["agents"][0]["agent"] = serde_json::json!("other")
        }),
        ("launch pending", |v| {
            v["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true)
        }),
        ("moved pane", |v| {
            v["snapshot"]["panes"][0]["pane_id"] = serde_json::json!("pane-b");
            v["snapshot"]["agents"][0]["pane_id"] = serde_json::json!("pane-b");
            v["snapshot"]["layouts"][0]["panes"][0]["pane_id"] = serde_json::json!("pane-b");
            v["snapshot"]["layouts"][0]["focused_pane_id"] = serde_json::json!("pane-b");
            v["snapshot"]["focused_pane_id"] = serde_json::json!("pane-b");
        }),
        ("moved tab", |v| {
            v["snapshot"]["tabs"][0]["tab_id"] = serde_json::json!("tab-b");
            v["snapshot"]["panes"][0]["tab_id"] = serde_json::json!("tab-b");
            v["snapshot"]["agents"][0]["tab_id"] = serde_json::json!("tab-b");
            v["snapshot"]["layouts"][0]["tab_id"] = serde_json::json!("tab-b");
            v["snapshot"]["focused_tab_id"] = serde_json::json!("tab-b");
        }),
        ("moved workspace", |v| {
            v["snapshot"]["workspaces"][0]["workspace_id"] = serde_json::json!("space-b");
            v["snapshot"]["tabs"][0]["workspace_id"] = serde_json::json!("space-b");
            v["snapshot"]["panes"][0]["workspace_id"] = serde_json::json!("space-b");
            v["snapshot"]["agents"][0]["workspace_id"] = serde_json::json!("space-b");
            v["snapshot"]["layouts"][0]["workspace_id"] = serde_json::json!("space-b");
            v["snapshot"]["focused_workspace_id"] = serde_json::json!("space-b");
        }),
    ];
    let before = fixture.bytes();
    for (name, change) in changes {
        let mut replacement = original.clone();
        change(&mut replacement);
        *fixture.payload.lock() = replacement;
        assert!(
            fixture
                .context
                .mutate(OrchestrationAction::RunAdopt {
                    label: "Must not commit".into(),
                })
                .await
                .is_err(),
            "{name}"
        );
        assert_eq!(fixture.bytes(), before, "{name}");
    }
    *fixture.payload.lock() = original;
    fixture.adopt().await;
}

#[tokio::test]
async fn postcommit_replacement_reports_uncertainty_and_preserves_committed_write() {
    let fixture = SocketFixture::new().await;
    let (ready, respond) = fixture.gate(1); // mutation's distinct postcheck
    let context = fixture.context.clone();
    let mutation = tokio::spawn(async move {
        context
            .mutate(OrchestrationAction::RunAdopt {
                label: "Committed once".into(),
            })
            .await
    });
    ready.await.unwrap();
    let committed = fixture.bytes().unwrap();
    let mut replacement = fixture.payload.lock().clone();
    replacement["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement");
    respond.send(Some(replacement)).unwrap();
    let error = mutation.await.unwrap().unwrap_err();
    assert_eq!(error.code, "caller_mismatch");
    assert!(
        error
            .message
            .contains("durable mutation may already be committed")
    );
    assert_eq!(fixture.bytes().unwrap(), committed);
    let state: serde_json::Value = serde_json::from_slice(&committed).unwrap();
    assert_eq!(state["runs"][0]["label"], "Committed once");
}

#[tokio::test]
async fn postchecked_snapshot_rejects_identity_replaced_during_core_observation() {
    let fixture = SocketFixture::new().await;
    fixture.adopt().await;
    let (ready, respond) = fixture.gate(1); // core runtime, after the precheck
    let context = fixture.context.clone();
    let read = tokio::spawn(async move { context.snapshot(None).await });
    ready.await.unwrap();
    let original = fixture.payload.lock().clone();
    fixture.payload.lock()["snapshot"]["agents"][0]["agent_session"]["value"] =
        serde_json::json!("replacement");
    respond.send(Some(original)).unwrap();
    assert_eq!(read.await.unwrap().unwrap_err().code, "caller_mismatch");
}

#[tokio::test]
async fn durable_change_before_wait_registration_is_delivered_without_lost_wake() {
    let fixture = SocketFixture::new().await;
    let run = fixture.adopt().await;
    let initial = fixture.context.snapshot(None).await.unwrap();
    assert!(!inbox_counts(&initial.messages, &run, 0).pending);
    // Commit between the consumer's empty observation and service subscription.
    fixture.send(&run, "edge-arrival");
    let next = next_snapshot(
        &fixture.context,
        &initial,
        tokio::time::Instant::now() + Duration::from_secs(10),
    )
    .await
    .unwrap()
    .unwrap();
    let counts = inbox_counts(&next.messages, &run, 0);
    assert!(counts.pending);
    assert_eq!(counts.through_seq, next.messages[0].seq);
    assert_eq!(next.messages[0].text, "edge-arrival");
    assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
    assert_eq!(fixture.context.own_run(&next).unwrap().run_id, run);
}

#[tokio::test]
async fn pending_inbox_and_controls_are_visible_in_first_completed_snapshot() {
    let fixture = SocketFixture::new().await;
    let run = fixture.adopt().await;
    for kind in [ReportKind::NeedsInput, ReportKind::Result] {
        fixture
            .context
            .mutate(OrchestrationAction::Report {
                message_id: format!("pending-{kind:?}"),
                kind,
                outcome: (kind == ReportKind::Result).then_some(ReportOutcome::Succeeded),
                summary: format!("Pending {kind:?}"),
                plan: None,
                to_run_id: None,
            })
            .await
            .unwrap();
    }
    fixture
        .context
        .mutate(OrchestrationAction::SubagentUpdate {
            subagent_id: "child".into(),
            parent_subagent_id: None,
            role: None,
            label: "Child".into(),
            status: SubagentStatus::Running,
            summary: None,
        })
        .await
        .unwrap();
    fixture.operator(OrchestrationAction::SubagentControl {
        run_id: run.clone(),
        subagent_id: "child".into(),
        op: SubagentOp::Cancel,
    });
    let snapshot = fixture.context.snapshot(None).await.unwrap();
    let before = fixture.bytes();
    assert!(inbox_counts(&snapshot.messages, &run, 0).pending);
    assert!(snapshot.messages.iter().any(|message| {
        message
            .report
            .as_ref()
            .is_some_and(|report| report.kind == ReportKind::NeedsInput)
    }));
    assert!(snapshot.messages.iter().any(|message| {
        message
            .report
            .as_ref()
            .is_some_and(|report| report.kind == ReportKind::Result)
    }));
    let controls = pending_controls(&snapshot.messages, &run, "child").unwrap();
    assert_eq!(controls.messages.len(), 1);
    let control: ControlPayload = serde_json::from_str(&controls.messages[0].text).unwrap();
    assert!(matches!(control._op, SubagentOp::Cancel));
    assert_eq!(fixture.bytes(), before);
    assert!(
        snapshot
            .messages
            .iter()
            .all(|message| message.stage == DeliveryStage::Stored)
    );
}

#[tokio::test]
async fn deadline_cancels_each_read_phase_and_arrivals_remain_durable_for_next_call() {
    for phase in 0..3 {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        fixture
            .context
            .mutate(OrchestrationAction::SubagentUpdate {
                subagent_id: "child".into(),
                parent_subagent_id: None,
                role: None,
                label: "Child".into(),
                status: SubagentStatus::Running,
                summary: None,
            })
            .await
            .unwrap();
        let initial = fixture.context.snapshot(None).await.unwrap();
        // A durable change makes service.wait finish immediately; isolate read cancellation.
        fixture
            .context
            .mutate(OrchestrationAction::Annotate {
                run_id: run.clone(),
                text: "Wake without inbox payload".into(),
            })
            .await
            .unwrap();
        let (ready, blocked) = fixture.gate(phase);
        let context = fixture.context.clone();
        tokio::time::pause();
        let read = tokio::spawn(async move {
            next_snapshot(
                &context,
                &initial,
                tokio::time::Instant::now() + Duration::from_millis(100),
            )
            .await
        });
        reach_gate_without_advancing_time(ready).await;
        fixture.send(&run, "deadline-arrival");
        fixture.operator(OrchestrationAction::SubagentControl {
            run_id: run.clone(),
            subagent_id: "child".into(),
            op: SubagentOp::Cancel,
        });
        let committed = fixture.bytes();
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::time::resume();
        assert!(read.await.unwrap().unwrap().is_none());
        drop(blocked);
        assert_eq!(fixture.bytes(), committed);
        let next = fixture.context.snapshot(None).await.unwrap();
        assert!(inbox_counts(&next.messages, &run, 0).pending);
        assert_eq!(
            next.messages
                .iter()
                .find(|m| m.text == "deadline-arrival")
                .unwrap()
                .stage,
            DeliveryStage::Stored
        );
        let controls = pending_controls(&next.messages, &run, "child").unwrap();
        assert_eq!(controls.messages.len(), 1);
        assert_eq!(controls.messages[0].stage, DeliveryStage::Stored);
    }
}

#[tokio::test]
async fn timeout_empty_fence_propagates_replacement_and_unavailable_endpoint() {
    for unavailable in [false, true] {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        let initial = fixture.context.snapshot(None).await.unwrap();
        fixture
            .context
            .mutate(OrchestrationAction::Annotate {
                run_id: run,
                text: "Wake".into(),
            })
            .await
            .unwrap();
        let committed = fixture.bytes();
        let (ready, blocked) = fixture.gate(0);
        let (fence_ready, fence) = fixture.gate(0);
        let context = fixture.context.clone();
        tokio::time::pause();
        let read = tokio::spawn(async move {
            next_snapshot(
                &context,
                &initial,
                tokio::time::Instant::now() + Duration::from_millis(100),
            )
            .await
        });
        reach_gate_without_advancing_time(ready).await;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::time::resume();
        fence_ready.await.unwrap();
        assert!(!read.is_finished());
        let mut replacement = fixture.payload.lock().clone();
        replacement["snapshot"]["boot_id"] = serde_json::json!("replacement");
        fence
            .send(if unavailable { None } else { Some(replacement) })
            .unwrap();
        let error = read.await.unwrap().unwrap_err();
        assert_eq!(
            error.code,
            if unavailable {
                "disconnected"
            } else {
                "caller_mismatch"
            }
        );
        assert_eq!(fixture.bytes(), committed);
        drop(blocked);
    }
}

#[tokio::test]
async fn caller_supplied_zero_and_short_wait_budgets_return_only_fenced_empty() {
    for budget in [Duration::ZERO, Duration::from_millis(20)] {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        let initial = fixture.context.snapshot(None).await.unwrap();
        let before = fixture.bytes();
        let deadline = tokio::time::Instant::now() + budget;
        tokio::time::pause();
        let context = fixture.context.clone();
        let read = tokio::spawn(async move { next_snapshot(&context, &initial, deadline).await });
        tokio::time::advance(budget + Duration::from_millis(1)).await;
        tokio::time::resume();
        assert!(read.await.unwrap().unwrap().is_none());
        let snapshot = fixture.context.snapshot(None).await.unwrap();
        assert!(!inbox_counts(&snapshot.messages, &run, 0).pending);
        assert!(
            pending_controls(&snapshot.messages, &run, "child")
                .unwrap()
                .messages
                .is_empty()
        );
        assert_eq!(fixture.bytes(), before);
    }
}

#[tokio::test]
async fn aborting_wait_observation_never_mutates_or_acknowledges_durable_state() {
    let fixture = SocketFixture::new().await;
    let run = fixture.adopt().await;
    let initial = fixture.context.snapshot(None).await.unwrap();
    fixture.send(&run, "unacked");
    let before = fixture.bytes();
    let (ready, blocked) = fixture.gate(0);
    let context = fixture.context.clone();
    let read = tokio::spawn(async move {
        next_snapshot(
            &context,
            &initial,
            tokio::time::Instant::now() + Duration::from_secs(10),
        )
        .await
    });
    ready.await.unwrap();
    read.abort();
    assert!(read.await.unwrap_err().is_cancelled());
    drop(blocked);
    assert_eq!(fixture.bytes(), before);
    let next = fixture.context.snapshot(None).await.unwrap();
    assert!(inbox_counts(&next.messages, &run, 0).pending);
    assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
}

#[tokio::test]
async fn watched_change_interrupts_registered_host_wait_without_timer_advance() {
    let fixture = SocketFixture::new().await;
    let run = fixture.adopt().await;
    let initial = fixture.context.snapshot(None).await.unwrap();
    let mut waiting = Box::pin(next_snapshot(
        &fixture.context,
        &initial,
        tokio::time::Instant::now() + Duration::from_secs(10),
    ));
    let pending =
        std::future::poll_fn(|cx| std::task::Poll::Ready(waiting.as_mut().poll(cx).is_pending()))
            .await;
    assert!(pending);
    fixture.send(&run, "watched-arrival");
    let next = waiting.await.unwrap().unwrap();
    assert!(inbox_counts(&next.messages, &run, 0).pending);
    assert_eq!(next.messages[0].text, "watched-arrival");
    assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
}

#[tokio::test]
async fn external_durable_change_is_observed_by_host_wait_and_not_acknowledged() {
    let fixture = SocketFixture::new().await;
    let run = fixture.adopt().await;
    let initial = fixture.context.snapshot(None).await.unwrap();
    let external = OrchestrationService::open(&fixture.configuration).unwrap();
    let mut waiting = Box::pin(next_snapshot(
        &fixture.context,
        &initial,
        tokio::time::Instant::now() + Duration::from_secs(10),
    ));
    let pending =
        std::future::poll_fn(|cx| std::task::Poll::Ready(waiting.as_mut().poll(cx).is_pending()))
            .await;
    assert!(pending);
    external
        .mutate(
            &Actor::Operator(OperatorOrigin::Browser),
            OrchestrationMutationRequest {
                session_id: "fixture".into(),
                expected_revision: Some(initial.revision),
                action: OrchestrationAction::MessageSend {
                    message_id: "external-arrival".into(),
                    to_run_id: run.clone(),
                    kind: MessageKind::Instruction,
                    text: "External arrival".into(),
                    in_reply_to: None,
                },
            },
        )
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::time::resume();
    let next = waiting.await.unwrap().unwrap();
    assert!(inbox_counts(&next.messages, &run, 0).pending);
    assert_eq!(next.messages[0].text, "External arrival");
    assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
}

#[tokio::test]
async fn same_socket_endpoint_process_replacement_invalidates_before_mutation() {
    let fixture = SocketFixture::new().await;
    let run = fixture.adopt().await;
    fixture.server.abort();
    // Await listener disposal rather than racing a second bind at the same path.
    while !fixture.server.is_finished() {
        tokio::task::yield_now().await;
    }
    std::fs::remove_file(fixture.root.join("herdr.sock")).unwrap();
    std::fs::write(
        fixture.root.join("payload.json"),
        serde_json::to_vec(&socket_payload()).unwrap(),
    )
    .unwrap();
    let child = EndpointChild::start(&fixture.root);
    let before = fixture.bytes();
    let error = fixture
        .context
        .mutate(OrchestrationAction::Annotate {
            run_id: run,
            text: "Must not commit".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "caller_mismatch");
    assert_eq!(fixture.bytes(), before);
    drop(child);
}
