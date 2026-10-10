use clap::{Parser, Subcommand};
use cockpit_core::orchestration::AgentCaller;
use cockpit_protocol::orchestration::{
    ActorRef, AgentKind, CloseReason, DeliveryStage, DispatchState, DispatchStep, Message,
    MessageKind, NativeProcessIdentity, NativeShellIdentity, OrchestrationAction,
    RetirementIdentity, RetirementState, RetirementTrigger, Run, RunKind, RunLocation,
    RunRetirement, RunStage,
};

use super::{
    CliError,
    args::{InboxArgs, RouteArgs, RunArgs, SubagentArgs, TaskArgs},
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

fn retirement_fixture() -> (Run, AgentCaller) {
    let process = NativeProcessIdentity {
        pid: 1234,
        start_ticks: 5678,
        kernel_boot_id: Some("kernel".into()),
    };
    let caller = AgentCaller {
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
        prepare_brief: String::new(),
        run_id: "worker".into(),
        kind: RunKind::Worker,
        label: "worker".into(),
        root_id: "root".into(),
        parent_run_id: Some("root".into()),
        task_id: Some("task".into()),
        attempt: 2,
        task_revision_at_propose: None,
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
        prepare_plan: None,
        init_receipt: None,
        work_plan: None,
        grants: vec![],
        last_report: None,
        result: None,
        annotations: vec![],
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
        bound_omp_session: Some("main".into()),
        bound_omp_process: Some(process),
        launch_shell_identity: Some(NativeShellIdentity {
            process: NativeProcessIdentity {
                pid: 4321,
                start_ticks: 5000,
                kernel_boot_id: Some("kernel".into()),
            },
            executable_device: "40".into(),
            executable_inode: "800".into(),
            argv_digest: "a".repeat(64),
        }),
        retirement: None,
        supersedes_run_id: None,
        created_at: "2026-10-07T00:00:00Z".into(),
        updated_at: "2026-10-07T00:00:00Z".into(),
    };
    (run, caller)
}

fn accept_retirement(run: &mut Run, caller: &AgentCaller) {
    run.stage = RunStage::Closed;
    run.close_reason = Some(CloseReason::Accepted);
    run.retirement = Some(RunRetirement {
        retirement_id: "retirement".into(),
        trigger: RetirementTrigger::Accept,
        result_message_id: "result".into(),
        task_revision: "revision".into(),
        identity: Some(RetirementIdentity {
            run_attempt: run.attempt,
            launch_attempt: 3,
            launch_tag: "launch".into(),
            endpoint_identity: caller.endpoint_identity.clone(),
            session_id: caller.session_id.clone(),
            workspace_id: caller.workspace_id.clone(),
            tab_id: caller.tab_id.clone(),
            pane_id: caller.pane_id.clone(),
            terminal_id: caller.terminal_id.clone().unwrap(),
            herdr_boot_id: caller.boot_id.clone(),
            omp_session_id: "main".into(),
            process: caller.process.clone().unwrap(),
            shell: run.launch_shell_identity.clone().unwrap(),
        }),
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
