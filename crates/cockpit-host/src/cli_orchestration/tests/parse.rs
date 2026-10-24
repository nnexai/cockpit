use std::path::PathBuf;

use clap::Parser;
use cockpit_protocol::orchestration::{
    DispatchTarget, OrchestrationAction, TaskStepScope,
};

use super::super::{
    CliError,
    args::{MAX_TEXT_BYTES, RecoveryArg, RunCommand, RunOperation, TaskCommand, InboxCommand},
    context::read_bounded,
    task_submission_error,
};
use super::{TestCli, TestCommand, parsed_task_action};

#[test]
fn task_creation_preserves_identity_prose_and_exact_relationship_fences() {
    let id = "08776d1f-6352-4aad-9c6b-48b2094c49b7";
    let action = parsed_task_action(&[
        "create",
        "--task-id",
        id,
        "--title",
        "Follow up",
        "--description",
        "Prose\nwith Unicode λ",
        "--depends-on",
        "first",
        "--depends-on",
        "second",
        "--follow-up-of",
        "source",
        "--doc-revision",
        "doc",
        "--source-revision",
        "source-rev",
    ])
    .unwrap();
    let encoded = serde_json::to_value(&action).unwrap();
    assert_eq!(encoded["task_id"], id);
    assert_eq!(encoded["description"], "Prose\nwith Unicode λ");
    assert_eq!(
        encoded["depends_on"],
        serde_json::json!(["first", "second"])
    );
    assert_eq!(encoded["follow_up_of"], "source");
    assert_eq!(encoded["expected_doc_revision"], "doc");
    assert_eq!(encoded["source_revision"], "source-rev");
    assert!(encoded.get("body").is_none());

    let generated = parsed_task_action(&["create", "--title", "Independent"]).unwrap();
    let encoded = serde_json::to_value(generated).unwrap();
    let generated_id = encoded["task_id"].as_str().unwrap();
    assert_eq!(
        uuid::Uuid::parse_str(generated_id)
            .unwrap()
            .get_version_num(),
        4
    );
    assert_eq!(encoded["depends_on"], serde_json::json!([]));
    assert!(encoded["follow_up_of"].is_null());
    let error = task_submission_error(
        CliError::new(
            "caller_mismatch",
            "durable mutation may already be committed",
        ),
        generated_id,
    );
    assert_eq!(error.code, "caller_mismatch");

    for args in [
        vec!["create", "--title", "x", "--depends-on", "other"],
        vec![
            "create",
            "--title",
            "x",
            "--follow-up-of",
            "source",
            "--doc-revision",
            "doc",
        ],
        vec!["create", "--title", "x", "--source-revision", "source-rev"],
    ] {
        assert!(parsed_task_action(&args).is_err());
    }
}

#[test]
fn task_update_distinguishes_omitted_prose_from_explicit_clear() {
    let cleared = parsed_task_action(&[
        "update",
        "task-id",
        "--revision",
        "full-rev",
        "--description",
        "",
    ])
    .unwrap();
    assert!(matches!(cleared, OrchestrationAction::TaskUpdate {
        task_id, expected_task_revision, title: None, description: Some(description), ..
    } if task_id == "task-id" && expected_task_revision == "full-rev" && description.is_empty()));
    let title_only =
        parsed_task_action(&["update", "task-id", "--revision", "rev", "--title", "New"]).unwrap();
    assert!(matches!(title_only, OrchestrationAction::TaskUpdate {
        description: None, title: Some(title), ..
    } if title == "New"));
    assert!(parsed_task_action(&["update", "task-id", "--revision", "rev"]).is_err());
}

#[test]
fn prerequisites_replacement_can_clear_and_requires_both_exact_fences() {
    let prefix = ["dependencies-set", "task-id", "--revision", "task-rev"];
    assert!(parsed_task_action(&prefix).is_err());
    let mut args = prefix.to_vec();
    args.extend(["--doc-revision", "doc-rev"]);
    let clear = parsed_task_action(&args).unwrap();
    assert!(matches!(clear, OrchestrationAction::TaskDependenciesSet {
        task_id, expected_task_revision, expected_doc_revision, depends_on, ..
    } if task_id == "task-id" && expected_task_revision == "task-rev"
        && expected_doc_revision == "doc-rev" && depends_on.is_empty()));
    args.extend(["--depends-on", "a", "--depends-on", "b"]);
    let set = parsed_task_action(&args).unwrap();
    assert!(matches!(set, OrchestrationAction::TaskDependenciesSet {
        depends_on, ..
    } if depends_on == ["a", "b"]));
}

#[test]
fn checklist_scope_and_destination_are_explicit_without_document_fences() {
    let check = [
        "step-set-checked",
        "task-id",
        "--revision",
        "task-rev",
        "--step-id",
        "step-id",
        "--checked",
        "false",
    ];
    assert!(parsed_task_action(&check).is_err());
    let mut args = check.to_vec();
    args.extend(["--scope", "subtree"]);
    let encoded = serde_json::to_value(parsed_task_action(&args).unwrap()).unwrap();
    assert_eq!(encoded["checked"], false);
    assert_eq!(encoded["scope"], "subtree");
    assert_eq!(encoded["expected_task_revision"], "task-rev");
    assert!(encoded.get("expected_doc_revision").is_none());
    assert!(
        parsed_task_action(&[
            "step-set-checked",
            "task-id",
            "--revision",
            "rev",
            "--step-id",
            "step-id",
            "--scope",
            "leaf",
        ])
        .is_err()
    );
    let leaf = parsed_task_action(&[
        "step-set-checked",
        "task-id",
        "--revision",
        "rev",
        "--step-id",
        "step-id",
        "--scope",
        "leaf",
        "--checked",
        "true",
    ])
    .unwrap();
    assert!(matches!(
        leaf,
        OrchestrationAction::TaskStepSetChecked {
            scope: TaskStepScope::Leaf,
            checked: true,
            ..
        }
    ));

    let add = parsed_task_action(&[
        "step-add",
        "task-id",
        "--revision",
        "rev",
        "--step-id",
        "new-id",
        "--parent-step-id",
        "parent",
        "--before-step-id",
        "sibling",
        "--title",
        "New",
    ])
    .unwrap();
    assert!(matches!(add, OrchestrationAction::TaskStepAdd {
        step_id, parent_step_id: Some(parent), before_step_id: Some(before), title, ..
    } if step_id == "new-id" && parent == "parent" && before == "sibling" && title == "New"));
    let move_to_top = parsed_task_action(&[
        "step-move",
        "task-id",
        "--revision",
        "rev",
        "--step-id",
        "existing",
    ])
    .unwrap();
    assert!(matches!(move_to_top, OrchestrationAction::TaskStepMove {
        parent_step_id: None, before_step_id: None, step_id, ..
    } if step_id == "existing"));
    let rename = parsed_task_action(&[
        "step-rename",
        "task-id",
        "--revision",
        "rev",
        "--step-id",
        "existing",
        "--title",
        "Renamed",
    ])
    .unwrap();
    assert!(matches!(rename, OrchestrationAction::TaskStepRename {
        step_id, title, ..
    } if step_id == "existing" && title == "Renamed"));
    let remove = parsed_task_action(&[
        "step-remove",
        "task-id",
        "--revision",
        "rev",
        "--step-id",
        "existing",
    ])
    .unwrap();
    assert!(matches!(remove, OrchestrationAction::TaskStepRemove {
        step_id, expected_task_revision, ..
    } if step_id == "existing" && expected_task_revision == "rev"));
}

#[test]
fn global_evidence_is_available_after_nested_commands() {
    let parsed = TestCli::try_parse_from([
        "test",
        "run",
        "report",
        "--kind",
        "initialized",
        "--message-id",
        "dedupe",
        "--summary",
        "Checkout inspected",
        "--harness",
        "generic",
        "--session-id",
        "actual conversation",
        "--herdr-session",
        "fixture",
        "--herdr-socket",
        "/tmp/fixture.sock",
        "--config",
        "/tmp/fixture.toml",
        "--json",
    ])
    .unwrap();
    let TestCommand::Run(args) = parsed.command else {
        panic!("expected run")
    };
    assert_eq!(args.common.session_id.as_deref(), Some("actual conversation"));
    assert!(args.common.json);
    assert_eq!(args.common.config, Some(PathBuf::from("/tmp/fixture.toml")));
}

#[test]
fn removed_aliases_are_rejected_and_message_ids_are_optional() {
    let cli = <TestCli as clap::CommandFactory>::command();
    let run = cli.find_subcommand("run").unwrap();
    for command in ["propose", "prepare", "execute", "bind", "bind-session", "retry-launch",
        "grant-prepare", "grant-execute", "attach", "child-register", "adopt", "retirement", "retirement-receipt"] {
        assert!(run.find_subcommand(command).is_none(), "{command} must not remain registered");
        assert!(TestCli::try_parse_from(["test", "run", command]).is_err());
    }
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "report",
            "--kind",
            "progress",
            "--summary",
            "x"
        ])
        .is_ok()
    );
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "message",
            "child",
            "--kind",
            "instruction",
            "--text",
            "x"
        ])
        .is_ok()
    );
}

#[test]
fn initialized_is_diagnostic_and_ready_is_not_a_report_kind() {
    let prefix = ["test", "run", "report", "--message-id", "message", "--summary", "Checkout inspected",
        "--harness", "generic", "--session-id", "actual"];
    let TestCommand::Run(args) = TestCli::try_parse_from(
        prefix.into_iter().chain(["--kind", "initialized"]),
    ).unwrap().command else { panic!("expected run"); };
    let super::super::args::RunOperation::Mutation(action) =
        args.command.operation(&args.common).unwrap() else { panic!("expected mutation"); };
    assert!(matches!(action, OrchestrationAction::Report {
        kind: cockpit_protocol::orchestration::ReportKind::Initialized, plan: None, ..
    }));
    assert!(TestCli::try_parse_from(
        prefix.into_iter().chain(["--kind", "ready", "--plan", "Old exact plan"]),
    ).is_err());
}

#[test]
fn management_requires_explicit_targets_and_exact_revision_arguments() {
    for command in ["accept", "send-back", "cancel", "reconcile", "resume"] {
        assert!(TestCli::try_parse_from(["test", "run", command]).is_err());
    }
    for (command, flag) in [("accept", "--task-revision"), ("send-back", "--text")] {
        assert!(TestCli::try_parse_from(["test", "run", command, "worker"]).is_err());
        assert!(TestCli::try_parse_from(["test", "run", command, "worker", flag, "exact"]).is_ok());
        assert!(TestCli::try_parse_from([
            "test", "run", command, "worker", flag, "exact", "--operator",
        ]).is_ok());
    }
    assert!(TestCli::try_parse_from(["test", "run", "cancel", "worker"]).is_ok());
    assert!(TestCli::try_parse_from([
        "test", "run", "message", "worker", "--kind", "answer", "--text", "Use the owned fixture.",
        "--message-id", "answer-1", "--in-reply-to", "question-1",
    ]).is_ok());
}

#[tokio::test]
async fn message_reply_pairing_is_rejected_before_endpoint_access() {
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "message",
            "worker",
            "--kind",
            "answer",
            "--text",
            "Use the documented checkout.",
            "--message-id",
            "answer-1",
        ])
        .is_err()
    );
    for (kind, question) in [
        ("answer", ""),
        ("answer", " \t "),
        ("instruction", "question-1"),
        ("cancel-request", "question-1"),
    ] {
        let parsed = TestCli::try_parse_from([
            "test",
            "run",
            "message",
            "worker",
            "--kind",
            kind,
            "--text",
            "Feedback",
            "--message-id",
            "message-1",
            "--in-reply-to",
            question,
        ])
        .unwrap();
        let TestCommand::Run(args) = parsed.command else {
            panic!("expected run")
        };
        assert_eq!(args.run().await.unwrap_err().code, "orchestration_usage");
    }
}

#[test]
fn reconcile_is_limited_to_explicit_uncertain_setup_recovery() {
    assert!(TestCli::try_parse_from(["test", "run", "reconcile", "worker"]).is_ok());
    assert!(TestCli::try_parse_from(["test", "run", "reconcile", "worker", "--operator"]).is_ok());
    let parsed = TestCli::try_parse_from([
        "test", "run", "reconcile", "worker", "--recovery", "accept-existing-worktree",
    ]).unwrap();
    let TestCommand::Run(args) = parsed.command else { panic!("expected run"); };
    assert!(matches!(args.command, RunCommand::Reconcile {
        run, recovery: Some(RecoveryArg::AcceptExistingWorktree),
    } if run == "worker"));
    assert!(TestCli::try_parse_from([
        "test", "run", "reconcile", "worker", "--recovery", "retry-environment",
    ]).is_err());
}

fn worker_flags<'a>() -> Vec<&'a str> {
    vec!["test", "run", "worker-start", "--operation-id", "operation", "--root", "root",
        "--task", "task", "--task-revision", "task-rev", "--doc-revision", "doc-rev",
        "--cleanup-policy", "owned-when-safe", "--brief", "Implement the assigned task"]
}

fn worker_action(target: &[&str]) -> OrchestrationAction {
    let mut flags = worker_flags();
    flags.extend_from_slice(target);
    let TestCommand::Run(args) = TestCli::try_parse_from(flags).unwrap().command else {
        panic!("expected run");
    };
    let RunOperation::WorkerStart(args) =
        args.command.operation(&args.common).unwrap() else { panic!("expected deferred start"); };
    args.action("root".into()).unwrap()
}

#[test]
fn worker_start_preserves_exact_operation_scope_and_cas_fences() {
    let encoded = serde_json::to_value(worker_action(&[
        "--space", "project", "--label", "worker",
    ])).unwrap();
    assert_eq!(encoded["action"], "worker_start");
    assert_eq!(encoded["operation_id"], "operation");
    assert_eq!(encoded["root_id"], "root");
    assert_eq!(encoded["task_id"], "task");
    assert_eq!(encoded["expected_task_revision"], "task-rev");
    assert_eq!(encoded["expected_doc_revision"], "doc-rev");
    assert_eq!(encoded["brief"], "Implement the assigned task");
    assert_eq!(encoded["cleanup_policy"], "owned_when_safe");
    assert_eq!(encoded["label"], "worker");
    for obsolete in ["prepare_brief", "parent_run_id", "supersedes_run_id", "plan_revision"] {
        assert!(encoded.get(obsolete).is_none());
    }
    for optional in ["--operation-id", "--task-revision", "--doc-revision", "--cleanup-policy", "--root"] {
        let mut flags = worker_flags();
        let index = flags.iter().position(|flag| *flag == optional).unwrap();
        flags.drain(index..index + 2);
        flags.extend(["--space", "project"]);
        let TestCommand::Run(args) = TestCli::try_parse_from(flags).unwrap().command else {
            panic!("expected run");
        };
        assert!(matches!(args.command.operation(&args.common).unwrap(), RunOperation::WorkerStart(_)));
    }
    let mut flags = worker_flags();
    let index = flags.iter().position(|flag| *flag == "--task").unwrap();
    flags.drain(index..index + 2);
    flags.extend(["--space", "project"]);
    assert!(TestCli::try_parse_from(flags).is_err());
    for obsolete in ["--parent", "--supersedes", "--plan-revision"] {
        let mut flags = worker_flags();
        flags.extend(["--space", "project", obsolete, "other"]);
        assert!(TestCli::try_parse_from(flags).is_err());
    }
    let mut flags = worker_flags();
    flags.extend(["--space", "project", "--operator"]);
    let TestCommand::Run(args) = TestCli::try_parse_from(flags).unwrap().command else { panic!("expected run") };
    assert!(args.common.operator);
}

#[tokio::test]
async fn operator_job_only_commands_refuse_before_endpoint_or_input_and_never_panic() {
    for tail in [
        vec!["context"], vec!["brief"], vec!["show", "--self"],
        vec!["register", "--operation-id", "operation"],
        vec!["answer", "--in-reply-to", "question", "--text", "Actual answer"],
        vec!["done", "--summary", "Completed"],
        vec!["report", "--kind", "progress", "--summary", "Update"],
        vec!["report", "--kind", "result", "--outcome", "succeeded", "--summary", "Completed"],
        vec!["resource-request", "--request-file", "/definitely/absent/input"],
        vec!["resource-build-register", "--registration-file", "/definitely/absent/input"],
    ] {
        let TestCommand::Run(args) = TestCli::try_parse_from(
            ["test", "run", "--operator"].into_iter().chain(tail),
        ).unwrap().command else { panic!("expected run") };
        assert_eq!(args.run().await.unwrap_err().code, "actor_forbidden");
    }
    for tail in [vec!["next"], vec!["wait"], vec!["list"], vec!["ack", "--through", "1"]] {
        let TestCommand::Inbox(args) = TestCli::try_parse_from(
            ["test", "inbox", "--operator"].into_iter().chain(tail),
        ).unwrap().command else { panic!("expected inbox") };
        assert_eq!(args.run().await.unwrap_err().code, "actor_forbidden");
    }
    for tail in [vec!["show", "--self"], vec!["inputs", "--self"]] {
        let TestCommand::Task(args) = TestCli::try_parse_from(
            ["test", "task", "--operator"].into_iter().chain(tail),
        ).unwrap().command else { panic!("expected task") };
        assert_eq!(args.run().await.unwrap_err().code, "actor_forbidden");
    }
}

#[test]
fn worker_start_targets_remain_exclusive_and_unfocused() {
    for targets in [
        ["--repository", "repo", "--path", "/tmp/project"],
        ["--repository", "repo", "--space", "project"],
        ["--path", "/tmp/project", "--space", "project"],
        ["--space-worktree", "project", "--repository", "repo"],
        ["--space-worktree", "project", "--path", "/tmp/project"],
        ["--space-worktree", "project", "--space", "project"],
    ] {
        let mut flags = worker_flags();
        flags.extend(targets);
        assert!(TestCli::try_parse_from(flags).is_err());
    }
    for target in [
        vec!["--repository", "repo", "--branch", "branch", "--base", "main",
            "--checkout-path", "/tmp/checkout", "--artifact", "https://forge/item",
            "--linked-artifact", "https://forge/related", "--task-name", "Task"],
        vec!["--path", "/tmp/project", "--task-name", "Task"],
    ] {
        let encoded = serde_json::to_value(worker_action(&target)).unwrap();
        assert_eq!(encoded["target"]["request"]["focus"], false);
    }
    let action = worker_action(&[
        "--space-worktree", "project", "--branch", "branch", "--base", "main",
    ]);
    assert!(matches!(action, OrchestrationAction::WorkerStart {
        target: DispatchTarget::SpaceWorktree {
            workspace_id, branch: Some(branch), base_ref: Some(base),
        }, ..
    } if workspace_id == "project" && branch == "branch" && base == "main"));
    for target in ["--path", "--space"] {
        for modifier in ["--branch", "--base"] {
            let mut flags = worker_flags();
            flags.extend([target, "project", modifier, "branch"]);
            assert!(TestCli::try_parse_from(flags).is_err());
        }
    }
}

#[test]
fn scope_update_preserves_live_run_exact_fences_and_optional_prose() {
    let prefix = ["scope-update", "task", "--operation-id", "operation", "--run", "worker",
        "--revision", "task-rev", "--doc-revision", "doc-rev"];
    assert!(parsed_task_action(&prefix).is_err());
    let mut flags = prefix.to_vec();
    flags.extend(["--description", ""]);
    let encoded = serde_json::to_value(parsed_task_action(&flags).unwrap()).unwrap();
    assert_eq!(encoded["action"], "task_scope_update");
    assert_eq!(encoded["operation_id"], "operation");
    assert_eq!(encoded["root_id"], "root");
    assert_eq!(encoded["task_id"], "task");
    assert_eq!(encoded["run_id"], "worker");
    assert_eq!(encoded["expected_task_revision"], "task-rev");
    assert_eq!(encoded["expected_doc_revision"], "doc-rev");
    assert_eq!(encoded["description"], "");
    assert!(encoded["title"].is_null());
    let mut flags = prefix.to_vec();
    flags.extend(["--title", "New scope"]);
    let encoded = serde_json::to_value(parsed_task_action(&flags).unwrap()).unwrap();
    assert_eq!(encoded["title"], "New scope");
    assert!(encoded["description"].is_null());
    for required in ["--operation-id", "--run", "--revision", "--doc-revision"] {
        let mut flags = prefix.to_vec();
        let index = flags.iter().position(|flag| *flag == required).unwrap();
        flags.drain(index..index + 2);
        flags.extend(["--title", "New scope"]);
        assert!(parsed_task_action(&flags).is_err(), "{required} is required");
    }
}

#[test]
fn limits_are_enforced_at_parser_and_input_boundary() {
    assert!(TestCli::try_parse_from(["test", "inbox", "wait", "--timeout", "3601"]).is_err());
    assert!(TestCli::try_parse_from(["test", "inbox", "list", "--limit", "101"]).is_err());
    assert_eq!(
        read_bounded(std::io::Cursor::new(vec![b'a'; MAX_TEXT_BYTES]))
            .unwrap()
            .len(),
        MAX_TEXT_BYTES
    );
    assert_eq!(
        read_bounded(std::io::Cursor::new(vec![b'a'; MAX_TEXT_BYTES + 1]))
            .unwrap_err()
            .code,
        "message_too_large"
    );
    assert!(read_bounded(std::io::Cursor::new(vec![0xff])).is_err());
    assert!(
        TestCli::try_parse_from([
            "test",
            "task",
            "create",
            "--title",
            "x",
            "--stdin",
            "--description-file",
            "description"
        ])
        .is_err()
    );
}

#[test]
fn literal_inputs_and_main_session_evidence_are_discoverable() {
    assert!(
        TestCli::try_parse_from([
            "test",
            "task",
            "create",
            "--title",
            "Task",
            "--description",
            "Literal description"
        ])
        .is_ok()
    );
    let mut flags = worker_flags();
    flags.extend(["--space", "workspace"]);
    assert!(TestCli::try_parse_from(flags).is_ok());
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "context", "--harness", "generic", "--session-id", "actual",
    ]).unwrap().command else { panic!("expected run"); };
    assert_eq!(args.common.harness.as_deref(), Some("generic"));
    assert_eq!(args.common.session_id.as_deref(), Some("actual"));
}

#[test]
fn durable_lifecycle_commands_preserve_explicit_operation_payloads() {
    let id = "81375f7e-a83a-4313-a415-39cc81ef24c2";
    let cases = [
        (vec!["start", "--operation-id", id, "--mode", "reuse-or-resume", "--space", "space",
            "--selected-root-id", "root", "--expected-project-key", "project"], "supervisor_start"),
        (vec!["resume", "run", "--operation-id", id,
            "--expected-attachment-epoch", "7"], "run_resume"),
        (vec!["register", "--operation-id", id], "run_register"),
    ];
    for (flags, expected) in cases {
        let TestCommand::Run(args) = TestCli::try_parse_from(
            ["test", "run"].into_iter().chain(flags),
        ).unwrap().command else { panic!("expected run"); };
        args.command.preflight(&args.common).unwrap();
        let super::super::args::RunOperation::Mutation(action) =
            args.command.operation(&args.common).unwrap() else { panic!("expected mutation"); };
        let encoded = serde_json::to_value(action).unwrap();
        assert_eq!(encoded["action"], expected);
        assert_eq!(encoded["operation_id"], id);
        assert!(encoded.get("auth").is_none());
        assert!(encoded.get("secret").is_none());
        if expected == "supervisor_start" {
            assert_eq!(encoded["mode"], "reuse_or_resume");
            assert_eq!(encoded["target"]["workspace_id"], "space");
            assert_eq!(encoded["selected_root_id"], "root");
            assert_eq!(encoded["expected_project_key"], "project");
        }
        if expected == "run_resume" {
            assert_eq!(encoded["expected_attachment_epoch"], 7);
            assert!(encoded.get("mode").is_none());
            assert!(encoded.get("decision_id").is_none());
        }
    }
    for flags in [
        vec!["start", "--mode", "another"],
        vec!["start", "--operation-id", id],
        vec!["resume", "run"],
        vec!["register"],
    ] {
        assert!(TestCli::try_parse_from(["test", "run"].into_iter().chain(flags)).is_err());
    }
    for flags in [
        vec!["resume", "run", "--operation-id", id, "--mode", "replace-existing"],
        vec!["resume", "run", "--operation-id", id, "--decision-id", "decision"],
    ] {
        assert!(TestCli::try_parse_from(["test", "run"].into_iter().chain(flags)).is_err());
    }
}

#[test]
fn supervisor_start_preserves_all_project_placements_and_no_focus() {
    for target in [
        vec!["--repository", "repo", "--branch", "branch", "--base", "main", "--checkout-path", "/tmp/checkout"],
        vec!["--path", "/tmp/project"],
        vec!["--space-worktree", "space", "--branch", "branch"],
        vec![],
    ] {
        let TestCommand::Run(args) = TestCli::try_parse_from(
            ["test", "run", "start", "--operation-id", "operation", "--mode", "another"]
                .into_iter().chain(target),
        ).unwrap().command else { panic!("expected run"); };
        let super::super::args::RunOperation::Mutation(action) =
            args.command.operation(&args.common).unwrap() else { panic!("expected mutation"); };
        let encoded = serde_json::to_value(action).unwrap();
        assert_eq!(encoded["mode"], "another");
        if let Some(request) = encoded["target"].get("request") {
            assert_eq!(request["focus"], false);
        }
    }
    assert!(TestCli::try_parse_from([
        "test", "run", "start", "--operation-id", "operation", "--mode", "another",
        "--space", "space", "--path", "/tmp/project",
    ]).is_err());
}

#[tokio::test]
async fn protocol_probe_needs_no_herdr_or_job_identity() {
    let TestCommand::Orchestration(args) = TestCli::try_parse_from([
        "test", "orchestration", "protocol", "--json",
    ]).unwrap().command else { panic!("expected protocol command"); };
    args.run().await.unwrap();
    assert!(TestCli::try_parse_from([
        "test", "orchestration", "protocol", "--credential", "secret",
    ]).is_err());
}

#[test]
fn structured_report_integration_and_resource_files_use_current_cli_names() {
    let path = std::env::temp_dir().join(format!("cockpit-cli-metadata-{}.json", uuid::Uuid::new_v4()));
    let holder = serde_json::json!({"run_id":"worker","attachment_id":"attachment","attachment_epoch":2});
    let cases = [
        ("resource-register", "--request-file", serde_json::json!({
            "operation_id":"creation","run_id":"worker",
            "resource":{"kind":"space","identity":{
                "creation_operation_id":"creation","endpoint_identity":"endpoint",
                "session_id":"physical","workspace_id":"space","checkout_path":"/tmp/project"
            }}
        }), "resource_register"),
        ("cleanup-finish", "--request-file", serde_json::json!({
            "operation_id":"cleanup","root_id":"root","run_id":"worker"
        }), "cleanup_finish"),
        ("integration-record", "--record-file", serde_json::json!({
            "operation_id":"integration","run_id":"worker","expected_result_message_id":"result",
            "expected_task_revision":"revision","integration":{"state":"pending"}
        }), "integration_record"),
        ("resource-request", "--request-file", serde_json::json!({
            "operation_id":"request","resource_key":"/tmp/build","holder":holder,
            "source_identity":{"checkout_key":"checkout","source_commit":"a".repeat(40),
                "source_tree":"b".repeat(40),"input_configuration":"release","toolchain_fingerprint":"rust"}
        }), "resource_request"),
        ("resource-release", "--release-file", serde_json::json!({
            "request_id":"request","expected_holder":holder
        }), "resource_release"),
        ("resource-build-register", "--registration-file", serde_json::json!({
            "request_id":"request","expected_holder":holder,
            "process":{"pid":123,"start_ticks":42,"kernel_boot_id":"boot"}
        }), "resource_build_register"),
    ];
    for (command, flag, payload, action_kind) in cases {
        std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
        let TestCommand::Run(args) = TestCli::try_parse_from([
            "test", "run", command, flag, path.to_str().unwrap(),
        ]).unwrap().command else { panic!("expected run"); };
        let super::super::args::RunOperation::Mutation(action) = args.command.operation(&args.common).unwrap()
            else { panic!("expected mutation"); };
        assert_eq!(serde_json::to_value(action).unwrap()["action"], action_kind);
    }
    std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
        "kind":"non_code","result_message_id":"result","task_revision":"revision",
        "artifacts":["artifact"],"focused_checks":[]
    })).unwrap()).unwrap();
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "report", "--kind", "result", "--outcome", "succeeded",
        "--message-id", "result", "--summary", "Artifact ready", "--source-output-file", path.to_str().unwrap(),
        "--harness", "generic", "--session-id", "actual",
        "--original-task-revision", "revision", "--original-claim-id", "claim",
    ]).unwrap().command else { panic!("expected run"); };
    let super::super::args::RunOperation::Mutation(action) = args.command.operation(&args.common).unwrap()
        else { panic!("expected mutation"); };
    assert!(matches!(&action, OrchestrationAction::Report {
        source_output: Some(cockpit_protocol::orchestration::SourceOutput::NonCode { .. }),
        original_task_revision: Some(revision), original_claim_id: Some(claim), ..
    } if revision == "revision" && claim == "claim"));
    std::fs::write(&path, vec![b' '; MAX_TEXT_BYTES + 1]).unwrap();
    assert_eq!(super::super::context::read_json_file::<serde_json::Value>(path.clone()).unwrap_err().code,
        "message_too_large");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn unassign_requires_exact_generation_and_operation_fences() {
    assert!(parsed_task_action(&["unassign", "task", "--operation-id", "operation"]).is_err());
    assert!(parsed_task_action(&["unassign", "task", "--expected-assignment-generation", "7"]).is_err());
    assert!(matches!(parsed_task_action(&[
        "unassign", "task", "--operation-id", "operation", "--expected-assignment-generation", "7",
    ]).unwrap(), OrchestrationAction::TaskUnassign {
        root_id, task_id, operation_id, expected_assignment_generation: 7,
    } if root_id == "root" && task_id == "task" && operation_id == "operation"));
}

#[test]
fn native_identity_and_delivery_paths_have_no_aliases() {
    for flag in ["--omp-session", "--omp-pid", "--omp-main-session", "--agent-kind", "--subagent-id"] {
        assert!(TestCli::try_parse_from(["test", "run", "context", flag, "old"]).is_err());
    }
    assert!(TestCli::try_parse_from(["test", "subagent", "controls"]).is_err());
    assert!(TestCli::try_parse_from(["test", "inbox", "woken", "--through", "1"]).is_err());
    assert!(TestCli::try_parse_from(["test", "inbox", "wait", "--with-retirement"]).is_err());
    assert!(TestCli::try_parse_from(["test", "inbox", "ack", "--through", "1"]).is_ok());
}

#[test]
fn schedule_defaults_and_explicit_scope_are_preserved() {
    let TestCommand::Task(args) = TestCli::try_parse_from([
        "test", "task", "schedule", "task",
    ]).unwrap().command else { panic!("expected task") };
    let TaskCommand::Schedule { task, operation_id, task_revision, doc_revision, launch } = args.command
        else { panic!("expected schedule") };
    assert_eq!(task, "task");
    assert!(operation_id.is_none() && task_revision.is_none() && doc_revision.is_none());
    let launch = launch.read().unwrap();
    assert!(launch.target.is_none(), "the domain chooses the source-owned isolated worktree");
    assert!(launch.brief.is_empty());
    assert_eq!(launch.cleanup_policy, cockpit_protocol::orchestration::WorkerCleanupPolicy::OwnedWhenSafe);
    let TestCommand::Task(args) = TestCli::try_parse_from([
        "test", "task", "schedule", "task", "--operation-id", "operation",
        "--task-revision", "task-rev", "--doc-revision", "doc-rev",
        "--space-worktree", "source", "--brief", "Extra constraint", "--label", "worker",
    ]).unwrap().command else { panic!("expected task") };
    let TaskCommand::Schedule { operation_id, task_revision, doc_revision, launch, .. } = args.command
        else { panic!("expected schedule") };
    assert_eq!(operation_id.as_deref(), Some("operation"));
    assert_eq!(task_revision.as_deref(), Some("task-rev"));
    assert_eq!(doc_revision.as_deref(), Some("doc-rev"));
    let launch = launch.read().unwrap();
    assert!(matches!(launch.target, Some(DispatchTarget::SpaceWorktree {
        workspace_id, branch: None, base_ref: None,
    }) if workspace_id == "source"));
    assert_eq!(launch.brief, "Extra constraint");
    assert_eq!(launch.label.as_deref(), Some("worker"));
    assert!(TestCli::try_parse_from(["test", "task", "schedule"]).is_err());
    assert!(TestCli::try_parse_from([
        "test", "task", "schedule", "task", "--space", "a", "--path", "/tmp/b",
    ]).is_err());
    assert!(TestCli::try_parse_from([
        "test", "task", "schedule", "task", "--brief", "x", "--brief-file", "brief",
    ]).is_err());
}

#[test]
fn schedule_cancel_and_restart_keep_explicit_targets_and_optional_ids() {
    for explicit in [false, true] {
        let mut flags = vec!["schedule-cancel", "task"];
        if explicit { flags.extend(["--operation-id", "operation"]); }
        let OrchestrationAction::TaskScheduleCancel { root_id, task_id, operation_id } =
            parsed_task_action(&flags).unwrap() else { panic!("expected cancel") };
        assert_eq!(root_id, "root");
        assert_eq!(task_id, "task");
        if explicit {
            assert_eq!(operation_id, "operation");
        } else {
            assert_eq!(uuid::Uuid::parse_str(&operation_id).unwrap().get_version_num(), 4);
        }
        let mut flags = vec!["test", "task", "restart", "task", "--reason", "Interrupted attempt"];
        if explicit { flags.extend(["--operation-id", "operation"]); }
        let TestCommand::Task(args) = TestCli::try_parse_from(flags).unwrap().command
            else { panic!("expected task") };
        assert!(matches!(args.command, TaskCommand::Restart { task, operation_id, reason }
            if task == "task" && operation_id.as_deref() == explicit.then_some("operation")
                && reason == "Interrupted attempt"));
    }
    for flags in [
        vec!["schedule-cancel"], vec!["restart", "task"], vec!["restart", "--reason", "x"],
    ] {
        assert!(TestCli::try_parse_from(["test", "task"].into_iter().chain(flags)).is_err());
    }
}

#[test]
fn replacement_brief_and_self_selection_need_no_conversation_flags() {
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "worker-replace", "worker", "--reason", "Lost attachment",
    ]).unwrap().command else { panic!("expected run") };
    assert!(args.common.harness.is_none() && args.common.session_id.is_none());
    assert!(matches!(args.command.operation(&args.common).unwrap(),
        RunOperation::WorkerReplace { run, operation_id: None, reason }
        if run == "worker" && reason == "Lost attachment"));
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "worker-replace", "worker", "--reason", "Replace safely",
        "--operation-id", "operation",
    ]).unwrap().command else { panic!("expected run") };
    assert!(matches!(args.command.operation(&args.common).unwrap(),
        RunOperation::WorkerReplace { operation_id: Some(id), .. } if id == "operation"));
    let TestCommand::Run(args) = TestCli::try_parse_from(["test", "run", "brief"]).unwrap().command
        else { panic!("expected run") };
    assert!(matches!(args.command.operation(&args.common).unwrap(), RunOperation::Brief));
    let TestCommand::Task(args) = TestCli::try_parse_from(["test", "task", "show", "--self"]).unwrap().command
        else { panic!("expected task") };
    assert!(matches!(args.command, TaskCommand::Show { task: None, self_task: true }));
    let TestCommand::Task(args) = TestCli::try_parse_from(["test", "task", "show", "task"]).unwrap().command
        else { panic!("expected task") };
    assert!(matches!(args.command, TaskCommand::Show { task: Some(task), self_task: false } if task == "task"));
    for flags in [
        vec!["test", "task", "show"],
        vec!["test", "task", "show", "task", "--self"],
        vec!["test", "run", "worker-replace", "worker"],
        vec!["test", "run", "worker-replace", "--reason", "x"],
        vec!["test", "run", "brief", "worker"],
    ] {
        assert!(TestCli::try_parse_from(flags).is_err());
    }
}

#[test]
fn task_inputs_selects_explicit_or_own_task_without_a_mutation_action() {
    for (flags, expected_task, expected_self) in [
        (vec!["test", "task", "inputs", "task-id"], Some("task-id"), false),
        (vec!["test", "task", "inputs", "--self"], None, true),
        (vec!["test", "task", "inputs"], None, false),
    ] {
        let TestCommand::Task(args) = TestCli::try_parse_from(flags).unwrap().command
            else { panic!("expected task") };
        let TaskCommand::Inputs { task, self_task } = &args.command
            else { panic!("expected inputs") };
        assert_eq!(task.as_deref(), expected_task);
        assert_eq!(*self_task, expected_self);
        assert_eq!(
            args.command.action("root".into()).unwrap_err().code,
            "orchestration_usage",
        );
    }
    let conflict = TestCli::try_parse_from([
        "test", "task", "inputs", "task-id", "--self",
    ]).unwrap_err();
    assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn done_preserves_truthful_check_statuses_and_source_alternatives() {
    use cockpit_protocol::orchestration::CheckStatus;
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "done", "--summary", "Checks failed", "--failed",
        "--check", "parser::cargo test parser::failed",
        "--check", "native::native smoke::not_run::No disposable compositor",
        "--artifact", "/tmp/output-a", "--artifact", "https://forge/output-b",
    ]).unwrap().command else { panic!("expected run") };
    let RunOperation::Done { message_id, summary, failed, checks, artifacts, source_output_file } =
        args.command.operation(&args.common).unwrap() else { panic!("expected done") };
    assert!(message_id.is_none() && failed && source_output_file.is_none());
    assert_eq!(summary, "Checks failed");
    assert_eq!(artifacts, ["/tmp/output-a", "https://forge/output-b"]);
    let outcomes = checks.into_iter().map(super::super::args::check_outcome).collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(outcomes[0].outcome, CheckStatus::Failed);
    assert_eq!(outcomes[0].command_or_scenario, "cargo test parser");
    assert_eq!(outcomes[1].outcome, CheckStatus::NotRun);
    assert_eq!(outcomes[1].limitation.as_deref(), Some("No disposable compositor"));
    assert!(outcomes.iter().all(|check| check.evidence.is_none()));
    let passed = super::super::args::check_outcome("parser::cargo test parser::passed".into()).unwrap();
    assert_eq!(passed.outcome, CheckStatus::Passed);
    for invalid in ["contract", "contract::command", "::command::passed", "contract::::passed",
        "contract::command::success"] {
        assert_eq!(super::super::args::check_outcome(invalid.into()).unwrap_err().code, "orchestration_usage");
    }
    for flags in [
        vec!["done", "--summary", "No evidence yet", "--failed"],
        vec!["done", "--summary", "Committed output", "--message-id", "result",
            "--source-output-file", "source.json"],
    ] {
        assert!(TestCli::try_parse_from(["test", "run"].into_iter().chain(flags)).is_ok());
    }
    for flags in [
        vec!["done"],
        vec!["done", "--summary", "x", "--source-output-file", "source.json"],
        vec!["report", "--kind", "result", "--summary", "x", "--outcome", "succeeded",
            "--source-output-file", "source.json"],
        vec!["done", "--summary", "x", "--message-id", "result", "--source-output-file", "source.json",
            "--check", "contract::command::passed"],
        vec!["done", "--summary", "x", "--message-id", "result", "--source-output-file", "source.json",
            "--artifact", "artifact"],
    ] {
        assert!(TestCli::try_parse_from(["test", "run"].into_iter().chain(flags)).is_err());
    }
}

#[test]
fn report_generates_one_message_identity_when_not_supplied() {
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "report", "--kind", "needs-input", "--summary", "Choose the outcome",
    ]).unwrap().command else { panic!("expected run") };
    let RunOperation::Mutation(OrchestrationAction::Report { message_id, summary, kind, .. }) =
        args.command.operation(&args.common).unwrap() else { panic!("expected report") };
    assert_eq!(uuid::Uuid::parse_str(&message_id).unwrap().get_version_num(), 4);
    assert_eq!(summary, "Choose the outcome");
    assert_eq!(kind, cockpit_protocol::orchestration::ReportKind::NeedsInput);
}

#[test]
fn root_answer_preserves_exact_question_and_human_text_without_target_selection() {
    let text = "Use the private fixture.\nKeep λ unchanged.";
    for explicit in [false, true] {
        let mut flags = vec!["test", "run", "answer", "--in-reply-to", "root-question", "--text", text];
        if explicit {
            flags.extend(["--message-id", "00000000-0000-4000-8000-000000000005"]);
        }
        let TestCommand::Run(args) = TestCli::try_parse_from(flags).unwrap().command
            else { panic!("expected run") };
        assert!(args.command.caller_required());
        args.command.preflight(&args.common).unwrap();
        let RunOperation::Answer { message_id, question, text: actual } =
            args.command.operation(&args.common).unwrap() else { panic!("expected root answer") };
        assert_eq!(question, "root-question");
        assert_eq!(actual, text);
        assert_eq!(message_id.as_deref(), explicit.then_some("00000000-0000-4000-8000-000000000005"));
    }
    for flags in [
        vec!["test", "run", "answer", "--text", text],
        vec!["test", "run", "answer", "--in-reply-to", "root-question"],
        vec!["test", "run", "answer", "worker", "--in-reply-to", "root-question", "--text", text],
    ] {
        assert!(TestCli::try_parse_from(flags).is_err());
    }
}

#[test]
fn root_answer_requires_nonempty_question_and_bounded_actual_human_text() {
    for (question, text) in [("", "answer"), (" \t ", "answer"), ("question", ""), ("question", " \n ")] {
        let TestCommand::Run(args) = TestCli::try_parse_from([
            "test", "run", "answer", "--in-reply-to", question, "--text", text,
        ]).unwrap().command else { panic!("expected run") };
        assert_eq!(args.command.operation(&args.common).err().unwrap().code, "orchestration_usage");
    }
    let exact = "λ".repeat(MAX_TEXT_BYTES / 2);
    let overflow = format!("{exact}a");
    for (text, valid) in [(exact.as_str(), true), (overflow.as_str(), false)] {
        let TestCommand::Run(args) = TestCli::try_parse_from([
            "test", "run", "answer", "--in-reply-to", "question", "--text", text,
        ]).unwrap().command else { panic!("expected run") };
        let result = args.command.operation(&args.common);
        if valid {
            assert!(matches!(result.unwrap(), RunOperation::Answer { text: actual, .. } if actual == text));
        } else {
            assert_eq!(result.err().unwrap().code, "message_too_large");
        }
    }
}

#[test]
fn inbox_next_wait_and_capture_options_are_bounded() {
    for wait in ["0", "3600"] {
        let TestCommand::Inbox(args) = TestCli::try_parse_from([
            "test", "inbox", "next", "--wait", wait, "--deliverable",
            "--capture-file", "/tmp/private/mail.json", "--json",
        ]).unwrap().command else { panic!("expected inbox") };
        assert!(matches!(args.command, InboxCommand::Next { wait: seconds, deliverable: true, capture_file: Some(path) }
            if seconds == wait.parse::<u64>().unwrap() && path == *"/tmp/private/mail.json"));
        assert!(args.common.json);
    }
    let TestCommand::Inbox(args) = TestCli::try_parse_from(["test", "inbox", "next"]).unwrap().command
        else { panic!("expected inbox") };
    assert!(matches!(args.command, InboxCommand::Next { wait: 0, deliverable: false, capture_file: None }));
    for invalid in ["-1", "3601", "18446744073709551616", "forever"] {
        assert!(TestCli::try_parse_from(["test", "inbox", "next", "--wait", invalid]).is_err());
    }
}

#[test]
fn typed_failures_and_conflicts_survive_cli_context() {
    for code in ["revision_conflict", "conversation_conflict", "assignment_conflict",
        "job_credential_invalid", "job_unauthorized", "message_id_conflict"] {
        let error = CliError::from(cockpit_core::InspectionError::new(code, "Exact failure"));
        assert_eq!(error.code, code);
        let error = task_submission_error(error, "retained-task");
        assert_eq!(error.code, code);
    }
}

#[test]
fn message_generates_one_identity_and_keeps_question_linkage() {
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "message", "worker", "--kind", "answer",
        "--text", "Use the owned fixture", "--in-reply-to", "question",
    ]).unwrap().command else { panic!("expected run") };
    args.command.preflight(&args.common).unwrap();
    let RunOperation::Mutation(OrchestrationAction::MessageSend { message_id, to_run_id, in_reply_to, .. }) =
        args.command.operation(&args.common).unwrap() else { panic!("expected message") };
    assert_eq!(uuid::Uuid::parse_str(&message_id).unwrap().get_version_num(), 4);
    assert_eq!(to_run_id, "worker");
    assert_eq!(in_reply_to.as_deref(), Some("question"));
}

#[test]
fn checklist_parse_allows_internally_resolved_task_revision() {
    let TestCommand::Task(args) = TestCli::try_parse_from([
        "test", "task", "step-set-checked", "task", "--step-id", "step",
        "--checked", "true", "--scope", "leaf",
    ]).unwrap().command else { panic!("expected task") };
    assert!(matches!(args.command, TaskCommand::StepSetChecked {
        task, step_id, checked: true, scope: super::super::args::StepScopeArg::Leaf,
    } if task.task == "task" && task.revision.is_none() && step_id == "step"));
}

#[test]
fn task_inputs_projection_keeps_direct_order_and_opaque_source_references() {
    use cockpit_protocol::orchestration::{Run, Task, TaskBoard};
    use serde_json::json;

    let task = |id: &str, depends_on: Vec<&str>| -> Task {
        serde_json::from_value(json!({
            "task_id": id, "title": id, "body": "private body",
            "description": "private description", "description_editable": true,
            "steps": [], "depends_on": depends_on, "checked": true,
            "line": 1, "task_revision": format!("revision-{id}"),
        })).unwrap()
    };
    let consumer = task("consumer", vec!["second", "missing", "first"]);
    let first = task("first", vec!["transitive"]);
    let mut second = task("second", vec![]);
    second.diagnostic = Some("Scope diagnostic".into());
    let dependency_state = json!({
        "state": "blocked",
        "unmet": [{"task_id": "missing", "reason": "missing"}],
        "problems": [],
    });
    let mut board: TaskBoard = serde_json::from_value(json!({
        "root_id": "root", "path": "private document", "doc_revision": "doc",
        "unidentified_items": 0, "diagnostics": [],
        "tasks": ([first, second].into_iter().map(|task| json!({
            "task": task, "lane": "accepted", "scheduled": false,
            "dependencies": {"state": "satisfied", "unmet": [], "problems": []},
        })).collect::<Vec<_>>()),
    })).unwrap();
    board.tasks.push(serde_json::from_value(json!({
        "task": &consumer, "lane": "blocked", "scheduled": false,
        "dependencies": &dependency_state,
    })).unwrap());
    let artifacts = json!(["../opaque?not=a-path", "sha256:not-a-hash", "https://example.invalid/ref"]);
    let run: Run = serde_json::from_value(json!({
        "session_id": "private session", "brief": "private brief",
        "run_id": "accepted-first", "kind": "worker", "label": "private label",
        "root_id": "root", "task_id": "first", "attempt": 2,
        "launch_config": {}, "stage": "closed", "close_reason": "accepted",
        "annotations": [], "created_at": "now", "updated_at": "now",
        "result": {"message_id": "result", "kind": "result", "outcome": "succeeded",
            "summary": "private summary", "plan": "private plan", "at": "now"},
        "source_output": {"kind": "non_code", "result_message_id": "result",
            "task_revision": "revision-first", "artifacts": artifacts, "focused_checks": []},
        "setup": {"checkout_path": "/actual/checkout", "base_commit": "base",
            "project_workspace_id": "source-space", "effects": ["private effect"],
            "warnings": ["private warning"]},
    })).unwrap();

    // Acceptance validity is supplied by the domain; this test covers projection only.
    let projected = super::super::output::task_inputs("root", &board, &consumer, &[&run]);
    assert_eq!(projected["dependency_state"], dependency_state);
    let dependencies = projected["dependencies"].as_array().unwrap();
    assert_eq!(dependencies.iter().map(|entry| entry["task_id"].as_str().unwrap())
        .collect::<Vec<_>>(), ["second", "missing", "first"]);
    assert!(dependencies[0]["accepted_run"].is_null());
    assert_eq!(dependencies[0]["diagnostics"][0], "Scope diagnostic");
    assert_eq!(dependencies[0]["diagnostics"][1],
        "No valid accepted Result/source reference matches the current prerequisite scope");
    assert!(dependencies[1]["accepted_run"].is_null());
    assert_eq!(dependencies[1]["diagnostics"][0],
        "Prerequisite is missing from the current canonical task document");
    let accepted = &dependencies[2]["accepted_run"];
    assert_eq!(accepted["source_output"]["artifacts"], artifacts);
    assert_eq!(accepted["result"], json!({
        "message_id": "result", "kind": "result", "outcome": "succeeded",
    }));
    assert_eq!(accepted["setup"], json!({
        "checkout_path": "/actual/checkout", "base_commit": "base",
        "project_workspace_id": "source-space",
    }));
    assert_eq!(accepted.as_object().unwrap().keys().map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>(),
        ["run_id", "attempt", "result", "source_output", "integration", "setup",
            "checkout", "project"].into_iter().collect());
    assert_eq!(accepted["source_output"].as_object().unwrap().keys().map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>(),
        ["kind", "result_message_id", "task_revision", "artifacts", "focused_checks"]
            .into_iter().collect());
}
