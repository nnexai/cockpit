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
        "ready",
        "--message-id",
        "dedupe",
        "--summary",
        "ready",
        "--plan",
        "Exact work plan",
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
fn operator_only_commands_and_missing_dedupe_are_rejected() {
    for command in ["grant-prepare", "grant-execute", "start"] {
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
fn management_requires_explicit_targets_and_exact_revision_arguments() {
    for command in [
        "prepare",
        "execute",
        "accept",
        "send-back",
        "cancel",
        "reconcile",
        "retry-launch",
    ] {
        assert!(TestCli::try_parse_from(["test", "run", command]).is_err());
    }
    for command in ["prepare", "execute", "accept", "send-back"] {
        assert!(TestCli::try_parse_from(["test", "run", command, "worker"]).is_err());
    }
    for (command, flag) in [
        ("prepare", "--plan-revision"),
        ("execute", "--plan-revision"),
        ("accept", "--task-revision"),
        ("send-back", "--text"),
    ] {
        assert!(TestCli::try_parse_from(["test", "run", command, "worker", flag, "exact"]).is_ok());
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                command,
                "worker",
                flag,
                "exact",
                "--operator"
            ])
            .is_err()
        );
    }
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "prepare",
            "worker",
            "--task-revision",
            "exact"
        ])
        .is_err()
    );
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "accept",
            "worker",
            "--plan-revision",
            "exact"
        ])
        .is_err()
    );
    assert!(TestCli::try_parse_from(["test", "run", "cancel", "worker"]).is_ok());
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
            "--in-reply-to",
            "question-1",
        ])
        .is_ok()
    );
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
fn recovery_commands_allow_only_scoped_explicit_worker_actions() {
    for command in ["reconcile", "retry-launch"] {
        assert!(TestCli::try_parse_from(["test", "run", command, "worker"]).is_ok());
        assert!(TestCli::try_parse_from(["test", "run", command, "worker", "--operator"]).is_err());
    }
    let parsed = TestCli::try_parse_from([
        "test",
        "run",
        "reconcile",
        "worker",
        "--recovery",
        "accept-existing-worktree",
    ])
    .unwrap();
    let TestCommand::Run(args) = parsed.command else {
        panic!("expected run")
    };
    assert!(matches!(args.command, RunCommand::Reconcile {
        run, recovery: Some(RecoveryArg::AcceptExistingWorktree),
    } if run == "worker"));
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "reconcile",
            "worker",
            "--recovery",
            "retry-environment",
        ])
        .is_err()
    );
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "retry-launch",
            "worker",
            "--recovery",
            "accept-existing-worktree",
        ])
        .is_err()
    );
}

#[test]
fn target_and_receipt_groups_are_exclusive() {
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "propose",
            "--task",
            "task",
            "--brief-file",
            "brief",
            "--path",
            "/tmp/a",
            "--space",
            "space"
        ])
        .is_err()
    );
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "propose",
            "--task",
            "task",
            "--brief-file",
            "brief",
            "--path",
            "/tmp/a",
            "--branch",
            "work"
        ])
        .is_err()
    );
    assert!(TestCli::try_parse_from(["test", "subagent", "control-done", "--seq", "1"]).is_err());
    assert!(
        TestCli::try_parse_from([
            "test",
            "subagent",
            "control-done",
            "--seq",
            "1",
            "--applied",
            "--failed",
            "failure"
        ])
        .is_err()
    );
}

#[test]
fn proposal_target_group_excludes_task_brief_and_repository_modifiers() {
    let parsed = TestCli::try_parse_from([
        "test",
        "run",
        "propose",
        "--task",
        "task",
        "--repository",
        "repository",
        "--brief",
        "Prepare only",
        "--branch",
        "feature",
        "--base",
        "main",
    ])
    .unwrap();
    let TestCommand::Run(args) = parsed.command else {
        panic!("expected run")
    };
    let RunCommand::Propose(proposal) = args.command else {
        panic!("expected proposal")
    };
    assert_eq!(proposal.task, "task");
    assert_eq!(proposal.repository.as_deref(), Some("repository"));
    assert_eq!(proposal.brief.as_deref(), Some("Prepare only"));
    assert_eq!(proposal.branch.as_deref(), Some("feature"));
    assert_eq!(proposal.base.as_deref(), Some("main"));
    for targets in [
        ["--repository", "repository", "--path", "/tmp/checkout"],
        ["--repository", "repository", "--space", "workspace"],
        ["--path", "/tmp/checkout", "--space", "workspace"],
        [
            "--space-worktree",
            "workspace",
            "--repository",
            "repository",
        ],
        ["--space-worktree", "workspace", "--path", "/tmp/checkout"],
        ["--space-worktree", "workspace", "--space", "workspace"],
    ] {
        let mut command = vec![
            "test",
            "run",
            "propose",
            "--task",
            "task",
            "--brief",
            "Prepare only",
        ];
        command.extend(targets);
        assert!(TestCli::try_parse_from(command).is_err());
    }
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "propose",
            "--task",
            "task",
            "--brief",
            "Prepare only",
        ])
        .is_err()
    );
}

#[test]
fn project_space_worktree_preserves_branch_base_and_restricts_modifiers() {
    let prefix = [
        "test",
        "run",
        "propose",
        "--task",
        "task",
        "--brief",
        "Read-only preparation",
    ];
    let mut command = prefix.to_vec();
    command.extend([
        "--space-worktree",
        "project",
        "--branch",
        "feature",
        "--base",
        "main",
    ]);
    let parsed = TestCli::try_parse_from(command).unwrap();
    let TestCommand::Run(args) = parsed.command else {
        panic!("expected run")
    };
    let RunCommand::Propose(proposal) = args.command else {
        panic!("expected proposal")
    };
    let OrchestrationAction::RunPropose { target, .. } = proposal.action().unwrap() else {
        panic!("expected proposal action")
    };
    assert!(matches!(target, DispatchTarget::SpaceWorktree {
        workspace_id, branch: Some(branch), base_ref: Some(base),
    } if workspace_id == "project" && branch == "feature" && base == "main"));
    for target in ["--space", "--path"] {
        for modifier in ["--branch", "--base"] {
            let mut command = prefix.to_vec();
            command.extend([target, "project", modifier, "feature"]);
            assert!(TestCli::try_parse_from(command).is_err());
        }
    }
    for modifier in [
        "--checkout-path",
        "--artifact",
        "--linked-artifact",
        "--task-name",
    ] {
        let mut command = prefix.to_vec();
        command.extend(["--space-worktree", "project", modifier, "value"]);
        assert!(
            TestCli::try_parse_from(command).is_err(),
            "{modifier} must reject --space-worktree"
        );
    }
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
    assert!(
        TestCli::try_parse_from([
            "test",
            "run",
            "propose",
            "--task",
            "task",
            "--space",
            "workspace",
            "--brief",
            "Prepare only"
        ])
        .is_ok()
    );
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
