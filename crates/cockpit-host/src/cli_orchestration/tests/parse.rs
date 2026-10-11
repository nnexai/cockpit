use std::path::PathBuf;

use clap::Parser;
use cockpit_protocol::orchestration::{
    DispatchTarget, NativeRefuseReason, NativeStopReceipt, OrchestrationAction, TaskStepScope,
};

use super::super::{
    CliError,
    args::{MAX_TEXT_BYTES, NativeRefuseReasonArg, RecoveryArg, RetirementReceiptArgs, RunCommand},
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
        "--omp-session",
        "native",
        "--agent-kind",
        "main",
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
    assert_eq!(args.common.omp_session.as_deref(), Some("native"));
    assert!(args.common.json);
    assert_eq!(args.common.config, Some(PathBuf::from("/tmp/fixture.toml")));
}

#[test]
fn removed_aliases_and_missing_dedupe_are_rejected() {
    let cli = <TestCli as clap::CommandFactory>::command();
    let run = cli.find_subcommand("run").unwrap();
    for command in ["propose", "prepare", "execute", "bind", "bind-session", "retry-launch",
        "grant-prepare", "grant-execute"] {
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
        .is_err()
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
        .is_err()
    );
}

#[test]
fn initialized_is_diagnostic_and_ready_is_not_a_report_kind() {
    let prefix = ["test", "run", "report", "--message-id", "message", "--summary", "Checkout inspected",
        "--omp-session", "main", "--agent-kind", "main"];
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
        ]).is_err());
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
    assert!(TestCli::try_parse_from(["test", "run", "reconcile", "worker", "--operator"]).is_err());
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
    let super::super::args::RunOperation::Mutation(action) =
        args.command.operation(&args.common).unwrap() else { panic!("expected mutation"); };
    action
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
    for required in ["--operation-id", "--task", "--task-revision", "--doc-revision", "--cleanup-policy"] {
        let mut flags = worker_flags();
        let index = flags.iter().position(|flag| *flag == required).unwrap();
        flags.drain(index..index + 2);
        flags.extend(["--space", "project"]);
        assert!(TestCli::try_parse_from(flags).is_err(), "{required} is required");
    }
    let mut flags = worker_flags();
    let index = flags.iter().position(|flag| *flag == "--root").unwrap();
    flags.drain(index..index + 2);
    flags.extend(["--space", "project"]);
    let TestCommand::Run(args) = TestCli::try_parse_from(flags).unwrap().command else {
        panic!("expected run");
    };
    assert!(args.command.operation(&args.common).is_err());
    for obsolete in ["--parent", "--supersedes", "--plan-revision", "--operator"] {
        let mut flags = worker_flags();
        flags.extend(["--space", "project", obsolete, "other"]);
        assert!(TestCli::try_parse_from(flags).is_err());
    }
}

#[test]
fn worker_start_targets_remain_explicit_exclusive_and_unfocused() {
    assert!(TestCli::try_parse_from(worker_flags()).is_err());
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
    for modifier in ["--checkout-path", "--artifact", "--linked-artifact", "--task-name"] {
        let mut flags = worker_flags();
        flags.extend(["--space-worktree", "project", modifier, "value"]);
        assert!(TestCli::try_parse_from(flags).is_err());
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
fn native_control_receipt_groups_are_exclusive() {
    assert!(TestCli::try_parse_from(["test", "subagent", "control-done", "--seq", "1"]).is_err());
    assert!(TestCli::try_parse_from([
        "test", "subagent", "control-done", "--seq", "1", "--applied", "--failed", "failure",
    ]).is_err());
}

#[test]
fn limits_are_enforced_at_parser_and_input_boundary() {
    assert!(TestCli::try_parse_from(["test", "inbox", "wait", "--timeout", "3601"]).is_err());
    assert!(
        TestCli::try_parse_from([
            "test",
            "subagent",
            "controls",
            "--id",
            "sub",
            "--timeout",
            "3601"
        ])
        .is_err()
    );
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
    let parsed = TestCli::try_parse_from([
        "test",
        "subagent",
        "controls",
        "--id",
        "child",
        "--wait",
        "--agent-kind",
        "subagent",
        "--subagent-id",
        "child",
        "--omp-session",
        "child-native",
        "--omp-main-session",
        "root-native",
    ])
    .unwrap();
    let TestCommand::Subagent(args) = parsed.command else {
        panic!("expected subagent")
    };
    assert_eq!(args.common.omp_main_session.as_deref(), Some("root-native"));
    assert!(args.common.validate_identity().is_ok());
}

#[test]
fn retirement_receipt_requires_one_typed_bounded_outcome() {
    let parsed = TestCli::try_parse_from([
        "test",
        "run",
        "retirement",
        "--omp-pid",
        "1234",
        "--wait",
        "--timeout",
        "30",
    ])
    .unwrap();
    let TestCommand::Run(args) = parsed.command else {
        panic!("expected run")
    };
    assert_eq!(args.common.omp_pid, Some(1234));
    assert!(matches!(
        args.command,
        RunCommand::Retirement {
            wait: true,
            timeout: 30
        }
    ));
    for reason in [
        "busy",
        "pending_messages",
        "async_jobs",
        "live_subagents",
        "editor_draft",
    ] {
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "retirement-receipt",
                "--retirement",
                "id",
                "--deferred",
                reason,
            ])
            .is_ok()
        );
    }
    for reason in ["user_activity", "native_refused"] {
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "retirement-receipt",
                "--retirement",
                "id",
                "--refused",
                "Explanation",
                "--refuse-reason",
                reason,
            ])
            .is_ok()
        );
    }
    for flags in [
        vec![],
        vec!["--shutdown-requested", "--deferred", "busy"],
        vec!["--refused", "user_activity"],
        vec!["--refuse-reason", "native_refused"],
    ] {
        let mut argv = vec!["test", "run", "retirement-receipt", "--retirement", "id"];
        argv.extend(flags);
        assert!(TestCli::try_parse_from(argv).is_err());
    }
    let action = RetirementReceiptArgs {
        retirement: "id".into(),
        shutdown_requested: false,
        deferred: None,
        refused: Some("user_activity appears here but has no authority".into()),
        refuse_reason: Some(NativeRefuseReasonArg::NativeRefused),
    }
    .action()
    .unwrap();
    assert!(matches!(
        action,
        OrchestrationAction::RetirementNativeReceipt {
            outcome: NativeStopReceipt::Refused {
                reason: NativeRefuseReason::NativeRefused,
                ..
            },
            ..
        }
    ));
    assert!(
        RetirementReceiptArgs {
            retirement: "id".into(),
            shutdown_requested: false,
            deferred: None,
            refused: Some("é".repeat(513)),
            refuse_reason: Some(NativeRefuseReasonArg::UserActivity),
        }
        .action()
        .is_err()
    );
}

#[tokio::test]
async fn retirement_commands_reject_missing_native_main_before_endpoint_access() {
    for mut argv in [
        vec!["test", "run", "retirement"],
        vec![
            "test",
            "run",
            "retirement-receipt",
            "--retirement",
            "id",
            "--shutdown-requested",
        ],
    ] {
        let TestCommand::Run(args) = TestCli::try_parse_from(&argv).unwrap().command else {
            panic!("expected run");
        };
        assert_eq!(args.run().await.unwrap_err().code, "orchestration_usage");
        argv.extend(["--omp-pid", "1234", "--agent-kind", "main"]);
        let TestCommand::Run(args) = TestCli::try_parse_from(argv).unwrap().command else {
            panic!("expected run");
        };
        assert_eq!(args.run().await.unwrap_err().code, "orchestration_usage");
    }
}

#[test]
fn durable_lifecycle_commands_preserve_explicit_operation_payloads() {
    let id = "81375f7e-a83a-4313-a415-39cc81ef24c2";
    let cases = [
        (vec!["start", "--operation-id", id, "--mode", "reuse-or-resume", "--space", "space",
            "--selected-root-id", "root", "--expected-project-key", "project"], "supervisor_start"),
        (vec!["resume", "run", "--operation-id", id, "--mode", "replace-existing",
            "--expected-attachment-epoch", "7", "--decision-id", "decision"], "run_resume"),
        (vec!["attach", "--operation-id", id, "--attachment-id", "attachment",
            "--expected-attachment-epoch", "7", "--main-session-id", "main"], "run_attach"),
        (vec!["child-register", "--operation-id", id, "--child-id", "child",
            "--child-session-id", "child-session", "--parent-child-id", "parent",
            "--agent-kind", "subagent", "--subagent-id", "child", "--omp-session", "child-session",
            "--omp-main-session", "main"], "job_child_register"),
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
            assert_eq!(encoded["mode"], "replace_existing");
            assert_eq!(encoded["expected_attachment_epoch"], 7);
            assert_eq!(encoded["decision_id"], "decision");
        }
    }
    for flags in [
        vec!["start", "--mode", "another"],
        vec!["start", "--operation-id", id],
        vec!["resume", "run", "--operation-id", id],
        vec!["attach", "--operation-id", id, "--attachment-id", "attachment", "--main-session-id", "main"],
    ] {
        assert!(TestCli::try_parse_from(["test", "run"].into_iter().chain(flags)).is_err());
    }
}

#[test]
fn child_registration_rejects_forged_child_context_before_endpoint_access() {
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "child-register", "--operation-id", "operation",
        "--child-id", "child", "--child-session-id", "child-session",
        "--agent-kind", "subagent", "--subagent-id", "different",
        "--omp-session", "child-session", "--omp-main-session", "main",
    ]).unwrap().command else { panic!("expected run"); };
    assert_eq!(args.command.preflight(&args.common).unwrap_err().code, "actor_forbidden");
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
    let descriptor = cockpit_host::transport::operations::orchestration::orchestration_protocol();
    assert_eq!(descriptor, serde_json::json!({
        "orchestration_protocol": 2, "store_schema": 2, "client_role": "host",
    }));
    args.run().await.unwrap();
    assert!(TestCli::try_parse_from([
        "test", "orchestration", "protocol", "--credential", "secret",
    ]).is_err());
}

#[tokio::test]
async fn terminal_cancel_receipt_is_explicit_child_only_and_not_action_authority() {
    let TestCommand::Subagent(args) = TestCli::try_parse_from([
        "test", "subagent", "control-done", "--seq", "3", "--applied", "--terminal-receipt",
        "--agent-kind", "main",
    ]).unwrap().command else { panic!("expected subagent"); };
    assert_eq!(args.run().await.unwrap_err().code, "actor_forbidden");
    for outcome in [vec!["--applied"], vec!["--failed", "native refusal"]] {
        let TestCommand::Subagent(args) = TestCli::try_parse_from(
            ["test", "subagent", "control-done", "--seq", "3", "--terminal-receipt",
                "--agent-kind", "subagent", "--subagent-id", "child", "--omp-session", "child-session",
                "--omp-main-session", "main"].into_iter().chain(outcome),
        ).unwrap().command else { panic!("expected subagent"); };
        assert!(matches!(args.command, super::super::args::SubagentCommand::ControlDone {
            terminal_receipt: true, seq: 3, ..
        }));
    }
    let TestCommand::Subagent(args) = TestCli::try_parse_from([
        "test", "subagent", "control-done", "--seq", "3", "--applied",
    ]).unwrap().command else { panic!("expected subagent"); };
    assert!(matches!(args.command, super::super::args::SubagentCommand::ControlDone {
        terminal_receipt: false, ..
    }));
}

#[test]
fn structured_report_integration_and_resource_files_use_current_cli_names() {
    let path = std::env::temp_dir().join(format!("cockpit-cli-metadata-{}.json", uuid::Uuid::new_v4()));
    let holder = serde_json::json!({"run_id":"worker","attachment_id":"attachment","attachment_epoch":2});
    let (mut retired, caller) = super::retirement_fixture();
    super::accept_retirement(&mut retired, &caller);
    let terminal_identity = retired.retirement.unwrap().identity.unwrap();
    let cases = [
        ("resource-register", "--request-file", serde_json::json!({
            "operation_id":"creation","run_id":"worker",
            "resource":{"kind":"worker_terminal","identity":terminal_identity}
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
        assert_eq!(args.command.physical_required(), command == "cleanup-finish");
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
        "--omp-session", "main", "--agent-kind", "main",
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
