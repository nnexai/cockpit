use clap::{Parser, Subcommand};
use cockpit_protocol::orchestration::{ActorRef, DeliveryStage, Message, MessageKind, OrchestrationAction};

use super::{CliError, args::{InboxArgs, ProtocolArgs, RouteArgs, RunArgs, TaskArgs}};

mod identity;
mod inbox_wait;
mod parse;

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
    Route(RouteArgs),
}

fn parsed_task_action(args: &[&str]) -> Result<OrchestrationAction, CliError> {
    let parsed = TestCli::try_parse_from(["test", "task"].into_iter().chain(args.iter().copied()))
        .map_err(|error| CliError::usage(error.to_string()))?;
    let TestCommand::Task(args) = parsed.command else { panic!("expected task") };
    args.command.action("root".into())
}

fn message(run: &str, seq: u64, kind: MessageKind, stage: DeliveryStage, text: &str) -> Message {
    Message {
        message_id: format!("message-{seq}"), to_run_id: run.into(), seq,
        from: ActorRef::Dispatcher, kind, text: text.into(), in_reply_to: None,
        report: None, stale: false, escalated_from: None, stage,
        created_at: "2026-10-05T00:00:00Z".into(), acked_at: None,
    }
}
