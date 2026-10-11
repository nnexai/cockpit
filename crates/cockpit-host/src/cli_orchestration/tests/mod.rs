use clap::{Parser, Subcommand};
use cockpit_core::orchestration::{AgentCaller, CallerProvenance, JobAuth, SecretBytes};
use cockpit_protocol::orchestration::{
    ActorRef, AgentKind, AttachmentLaunchMode, AttachmentState, CheckoutIdentity, CloseReason, DeliveryStage,
    DirectoryIdentity, DispatchState, DispatchStep, ExecutionAttachment, Message, MessageKind,
    NativeProcessIdentity, NativeShellIdentity, OrchestrationAction, ProjectIdentity, ProjectSource,
    RetirementIdentity, RetirementState, RetirementTrigger, Run, RunKind, RunLocation,
    RunRetirement, RunStage,
};

use super::{
    CliError,
    args::{InboxArgs, ProtocolArgs, RouteArgs, RunArgs, SubagentArgs, TaskArgs},
};

mod identity;
mod inbox_wait;
mod parse;
mod retirement_wait;
mod socket;

#[derive(Debug, Parser)]
struct TestCli {
    #[command(subcommand)]
    command: TestCommand,
}
#[derive(Debug, Subcommand)]
enum TestCommand {
    Orchestration(ProtocolArgs),
    Task(TaskArgs),
    Run(RunArgs),
    Inbox(InboxArgs),
    Subagent(SubagentArgs),
    Route(RouteArgs),
}

fn parsed_task_action(args: &[&str]) -> Result<OrchestrationAction, CliError> {
    let parsed = TestCli::try_parse_from(["test", "task"].into_iter().chain(args.iter().copied()))
        .map_err(|error| CliError::usage(error.to_string()))?;
    let TestCommand::Task(args) = parsed.command else {
        panic!("expected task")
    };
    args.command.action("root".into())
}

fn message(run: &str, seq: u64, kind: MessageKind, stage: DeliveryStage, text: &str) -> Message {
    Message {
        message_id: format!("message-{seq}"),
        to_run_id: run.into(),
        seq,
        from: ActorRef::Dispatcher,
        kind,
        text: text.into(),
        in_reply_to: None,
        report: None,
        stale: false,
        escalated_from: None,
        from_subagent_id: None,
        stage,
        woken_omp_session: None,
        created_at: "2026-10-05T00:00:00Z".into(),
        acked_at: None,
    }
}

fn test_job_auth() -> JobAuth {
    JobAuth {
        credential_id: "00000000-0000-4000-8000-000000000001".into(),
        secret: SecretBytes::new([7; 32]),
        attachment_id: "00000000-0000-4000-8000-000000000002".into(),
        attachment_epoch: 1,
    }
}

fn test_credential_record(run: &Run, auth: &JobAuth, role: &str) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    serde_json::json!({
        "credential_id": auth.credential_id,
        "run_id": run.run_id,
        "root_id": run.root_id,
        "role": role,
        "subject_child_id": null,
        "attachment_id": auth.attachment_id,
        "attachment_epoch": auth.attachment_epoch,
        "secret_digest": format!("{:x}", Sha256::digest([7_u8; 32])),
        "state": "active",
        "issued_at": "2026-10-07T00:00:00Z",
    })
}

fn test_checkout_claim(run: &Run) -> serde_json::Value {
    let attachment = run.attachment.as_ref().unwrap();
    let holder = serde_json::json!({
        "run_id": run.run_id,
        "attachment_id": attachment.attachment_id,
        "attachment_epoch": attachment.epoch,
    });
    serde_json::json!({
        "claim_id": run.checkout_claim_id,
        "checkout": run.checkout,
        "revision": 1,
        "state": "held",
        "holder": holder,
        "transition_operation_id": null,
    })
}

fn retirement_fixture() -> (Run, AgentCaller) {
    let process = NativeProcessIdentity {
        pid: 1234,
        start_ticks: 5678,
        kernel_boot_id: Some("kernel".into()),
    };
    let caller = AgentCaller {
        auth: test_job_auth(),
        provenance: Some(CallerProvenance {
            agent_kind: Some(AgentKind::Main),
            child_id: None,
            main_session_id: Some("main".into()),
            native_session_id: Some("main".into()),
            process: Some(process.clone()),
        }),
        endpoint_identity: "endpoint".into(),
        session_id: "fixture".into(),
        workspace_id: "space".into(),
        tab_id: "tab".into(),
        pane_id: "pane".into(),
        boot_id: Some("boot".into()),
        terminal_id: Some("terminal".into()),
        native_session_id: Some("main".into()),
        actual_agent_kind: Some("omp".into()),
        env_run: Some(("worker".into(), 2)),
        omp_session_id: Some("main".into()),
        main_omp_session_id: None,
        agent_kind: Some(AgentKind::Main),
        subagent_id: None,
        process: Some(process.clone()),
    };
    let run = Run {
        session_id: "fixture".into(),
        brief: String::new(),
        run_id: "worker".into(),
        kind: RunKind::Worker,
        label: "worker".into(),
        root_id: "root".into(),
        parent_run_id: Some("root".into()),
        task_id: Some("task".into()),
        attempt: 2,
        task_revision_at_start: None,
        stage: RunStage::Working,
        close_reason: None,
        dispatch: Some(DispatchState {
            launch_tag: Some("launch".into()),
            endpoint_identity: Some("endpoint".into()),
            recovery: None,
            agent_started: true,
            step: DispatchStep::Launched,
            launch_attempt: 3,
            error: None,
            updated_at: "2026-10-07T00:00:00Z".into(),
        }),
        target: None,
        setup: None,
        cleanup_policy: Some(cockpit_protocol::orchestration::WorkerCleanupPolicy::OwnedWhenSafe),
        last_report: None,
        result: None,
        source_output: None,
        integration: Default::default(),
        annotations: vec![],
        project: Some(ProjectIdentity {
            key: "test-project".into(),
            machine_id: "test-machine".into(),
            source: ProjectSource::Directory { root: "/tmp/ck-host-worker".into() },
            repository_id: None,
            source_workspace_id: None,
        }),
        checkout: Some(CheckoutIdentity {
            key: "test-checkout".into(),
            machine_id: "test-machine".into(),
            path: "/tmp/ck-host-worker".into(),
            directory_identity: DirectoryIdentity { device: "1".into(), inode: "2".into() },
            git: None,
        }),
        attachment: Some(ExecutionAttachment {
            attachment_id: caller.auth.attachment_id.clone(),
            epoch: caller.auth.attachment_epoch,
            state: AttachmentState::Connected,
            launch_mode: Some(AttachmentLaunchMode::ShellAgent),
            location: Some(RunLocation {
                boot_id: caller.boot_id.clone(),
                terminal_id: caller.terminal_id.clone(),
                native_session_id: caller.native_session_id.clone(),
                endpoint_identity: caller.endpoint_identity.clone(),
                session_id: caller.session_id.clone(),
                workspace_id: caller.workspace_id.clone(),
                tab_id: caller.tab_id.clone(),
                pane_id: caller.pane_id.clone(),
                launch_tag: "launch".into(),
            }),
            main_session_id: Some("main".into()),
            process: Some(process),
            shell: Some(NativeShellIdentity {
                process: NativeProcessIdentity {
                    pid: 4321,
                    start_ticks: 5000,
                    kernel_boot_id: Some("kernel".into()),
                },
                executable_device: "40".into(),
                executable_inode: "800".into(),
                argv_digest: "a".repeat(64),
            }),
            machine_id: None,
            launch_attempt: 3,
            updated_at: "2026-10-07T00:00:00Z".into(),
        }),
        checkout_claim_id: Some("00000000-0000-4000-8000-000000000003".into()),
        resume: None,
        retirement: None,
        created_at: "2026-10-07T00:00:00Z".into(),
        updated_at: "2026-10-07T00:00:00Z".into(),
    };
    (run, caller)
}

fn accept_retirement(run: &mut Run, caller: &AgentCaller) {
    run.stage = RunStage::Closed;
    run.close_reason = Some(CloseReason::Accepted);
    let output = cockpit_protocol::orchestration::SourceOutput::NonCode {
        result_message_id: "result".into(),
        task_revision: run.task_revision_at_start.clone().unwrap_or_else(|| "revision".into()),
        artifacts: vec![],
        focused_checks: vec![],
    };
    run.result = Some(cockpit_protocol::orchestration::Report {
        message_id: "result".into(),
        kind: cockpit_protocol::orchestration::ReportKind::Result,
        outcome: Some(cockpit_protocol::orchestration::ReportOutcome::Succeeded),
        summary: "Fixture non-code result".into(),
        plan: None,
        source_output: Some(output.clone()),
        at: run.updated_at.clone(),
    });
    run.source_output = Some(output);
    run.integration = cockpit_protocol::orchestration::Integration::NotApplicable {
        reason: "Non-code fixture output".into(), result_message_id: "result".into(),
    };
    run.attachment.as_mut().unwrap().state = AttachmentState::Quiescing;
    let attachment = run.attachment.as_ref().unwrap();
    run.retirement = Some(RunRetirement {
        retirement_id: "retirement".into(),
        trigger: RetirementTrigger::Accept,
        result_message_id: "result".into(),
        task_revision: run.task_revision_at_start.clone().unwrap_or_else(|| "revision".into()),
        identity: Some(RetirementIdentity {
            attachment_id: Some(attachment.attachment_id.clone()),
            attachment_epoch: Some(attachment.epoch),
            launch_mode: attachment.launch_mode,
            run_attempt: run.attempt,
            launch_attempt: attachment.launch_attempt,
            launch_tag: attachment.location.as_ref().unwrap().launch_tag.clone(),
            endpoint_identity: caller.endpoint_identity.clone(),
            session_id: caller.session_id.clone(),
            workspace_id: caller.workspace_id.clone(),
            tab_id: caller.tab_id.clone(),
            pane_id: caller.pane_id.clone(),
            terminal_id: caller.terminal_id.clone().unwrap(),
            herdr_boot_id: caller.boot_id.clone(),
            omp_session_id: attachment.main_session_id.clone().unwrap(),
            process: attachment.process.clone().unwrap(),
            shell: attachment.shell.clone().unwrap(),
        }),
        quiescent_stop_at: None,
        state: RetirementState::NativeStopOffered {
            offered_at: "2026-10-07T00:00:01Z".into(),
        },
        created_at: "2026-10-07T00:00:01Z".into(),
        updated_at: "2026-10-07T00:00:01Z".into(),
    });
}

#[test]
#[ignore = "owned endpoint responder, invoked only by the replacement test"]
fn socket_endpoint_responder_process() {
    use std::io::{BufRead, BufReader, Write};
    let socket = std::env::var_os("CK_HOST_TEST_SOCKET").unwrap();
    let payload = std::env::var_os("CK_HOST_TEST_PAYLOAD").unwrap();
    let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
    println!("CK_HOST_ENDPOINT_READY");
    std::io::stdout().flush().unwrap();
    for connection in listener.incoming() {
        let mut reader = BufReader::new(connection.unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            continue;
        }
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&payload).unwrap()).unwrap();
        let response = format!(
            "{}\n",
            serde_json::json!({
                "id": request["id"], "result": result,
            })
        );
        let _ = reader.into_inner().write_all(response.as_bytes());
    }
}
