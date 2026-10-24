use super::super::caller::resolve_conversation;

#[test]
fn explicit_non_omp_identity_needs_no_journal_and_rejects_conflicting_sources() {
    let missing = std::path::Path::new("/definitely/absent/session-file");
    let identity = resolve_conversation(Some("other".into()), Some("opaque/session".into()),
        None, None, Some(missing)).unwrap().unwrap();
    assert_eq!(identity.harness, "other");
    assert_eq!(identity.session_id, "opaque/session");
    assert_eq!(resolve_conversation(None, Some("id".into()), None, None, None)
        .unwrap_err().code, "orchestration_usage");
    for (harness, session) in [("omp", "id"), ("other", "stale")] {
        assert_eq!(resolve_conversation(Some(harness.into()), Some(session.into()),
            Some("other".into()), Some("id".into()), None).unwrap_err().code, "conversation_conflict");
    }
    for invalid in [String::new(), "a".repeat(513), "bad\nidentity".into()] {
        assert_eq!(resolve_conversation(Some("other".into()), Some(invalid), None, None, None)
            .unwrap_err().code, "conversation_invalid");
    }
}

#[test]
fn conversation_metadata_is_optional_without_a_session_source() {
    for (explicit, inherited) in [
        (None, None),
        (Some("other".into()), None),
        (Some("omp".into()), None),
        (None, Some("omp".into())),
    ] {
        assert_eq!(resolve_conversation(explicit, None, inherited, None, None).unwrap(), None);
    }
    assert_eq!(resolve_conversation(None, None, None, None,
        Some(std::path::Path::new("/definitely/absent/session-file"))).unwrap(), None);
    assert_eq!(resolve_conversation(Some("other".into()), None, Some("omp".into()), None, None)
        .unwrap_err().code, "conversation_conflict");
    for invalid in [String::new(), "a".repeat(513), "bad\nharness".into()] {
        assert_eq!(resolve_conversation(Some(invalid), None, None, None, None)
            .unwrap_err().code, "conversation_invalid");
    }
    assert_eq!(resolve_conversation(None, None, None, Some("session".into()), None)
        .unwrap_err().code, "orchestration_usage");
}

#[test]
fn omp_header_is_checked_against_explicit_and_environment_identity() {
    let root = std::env::temp_dir().join(format!("cockpit-cli-header-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let file = root.join("unrelated-name.jsonl");
    std::fs::write(&file, b"{\"type\":\"title\"}\n{\"type\":\"session\",\"id\":\"actual\"}\n").unwrap();
    assert_eq!(resolve_conversation(Some("omp".into()), None, None, None, Some(&file)).unwrap().unwrap().session_id, "actual");
    assert_eq!(resolve_conversation(Some("omp".into()), Some("actual".into()),
        Some("omp".into()), Some("actual".into()), Some(&file)).unwrap().unwrap().session_id, "actual");
    assert_eq!(resolve_conversation(Some("omp".into()), Some("stale".into()), None, None, Some(&file)).unwrap_err().code,
        "conversation_conflict");
    assert_eq!(resolve_conversation(None, None, Some("omp".into()), Some("stale".into()), Some(&file)).unwrap_err().code,
        "conversation_conflict");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn omp_explicit_identity_does_not_hide_header_failures() {
    let root = std::env::temp_dir().join(format!("cockpit-cli-invalid-header-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let malformed = root.join("malformed.jsonl");
    std::fs::write(&malformed, b"PRIVATE invalid header\n").unwrap();
    let invalid_id = root.join("invalid-id.jsonl");
    std::fs::write(&invalid_id, b"{\"type\":\"session\",\"id\":\"\"}\n").unwrap();
    let missing = root.join("missing.jsonl");
    for path in [&root, &missing, &malformed] {
        for session in [None, Some("actual".into())] {
            let error = resolve_conversation(Some("omp".into()), session,
                None, None, Some(path)).unwrap_err();
            assert_eq!(error.code, "session_header_invalid");
            assert!(!error.message.contains("PRIVATE"));
        }
    }
    assert_eq!(resolve_conversation(Some("omp".into()), Some("actual".into()),
        None, None, Some(&invalid_id)).unwrap_err().code, "conversation_invalid");
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "private CLI environment child, launched by the no-extension boundary test"]
async fn neutral_cli_environment_child() {
    use clap::Parser;
    use super::{TestCli, TestCommand};
    use super::super::context::Context;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use super::super::output::{capture_directory, default_capture_file};
    let TestCommand::Run(initial) = TestCli::try_parse_from([
        "test", "run", "context", "--json",
    ]).unwrap().command else { panic!("expected run") };
    let context = Context::open(&initial.common, true).await.unwrap();
    let cockpit_core::orchestration::Actor::Agent(caller) = context.actor.as_ref().unwrap() else {
        panic!("expected private credential")
    };
    let run_id = context.service.job_role(caller).unwrap().0.run_id;
    let directory = capture_directory(&caller.auth, &run_id).unwrap();
    initial.run().await.unwrap();
    assert!(!directory.exists(), "context only derives its recovery pointer");
    for field in 0..4 {
        let mut auth = caller.auth.clone();
        let mut other_run_id = run_id.clone();
        match field {
            0 => other_run_id = uuid::Uuid::new_v4().to_string(),
            1 => auth.attachment_id = uuid::Uuid::new_v4().to_string(),
            2 => auth.attachment_epoch += 1,
            _ => auth.credential_id = uuid::Uuid::new_v4().to_string(),
        }
        let other = capture_directory(&auth, &other_run_id).unwrap();
        assert_ne!(other, directory, "each issued identity field binds the spool");
        assert!(!other.exists(), "derivation never creates a spool");
    }
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let public = home.join("public-parent");
    std::fs::create_dir(&public).unwrap();
    std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
    let failed_capture = public.join("mail.json");
    let TestCommand::Inbox(failed) = TestCli::try_parse_from([
        "test", "inbox", "next", "--wait", "0", "--deliverable",
        "--capture-file", failed_capture.to_str().unwrap(),
    ]).unwrap().command else { panic!("expected inbox") };
    assert_eq!(failed.run().await.unwrap_err().code, "inbox_capture");
    let unread = context.snapshot(None).await.unwrap();
    assert!(unread.messages.iter().all(|message|
        message.stage == cockpit_protocol::orchestration::DeliveryStage::Read
        && message.acked_at.is_none()), "failed capture never ACKs even the filtered prefix");
    assert!(!failed_capture.exists());
    assert!(!directory.exists(), "explicit failure does not create the default spool");
    symlink(&home, &directory).unwrap();
    assert_eq!(default_capture_file(&caller.auth, &run_id).unwrap_err().code, "inbox_capture");
    assert_eq!(std::fs::read_link(&directory).unwrap(), home);
    std::fs::remove_file(&directory).unwrap();
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(default_capture_file(&caller.auth, &run_id).unwrap_err().code, "inbox_capture");
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    std::fs::remove_dir(&directory).unwrap();
    let TestCommand::Inbox(next) = TestCli::try_parse_from([
        "test", "inbox", "next", "--wait", "0", "--deliverable", "--json",
    ]).unwrap().command else { panic!("expected inbox") };
    next.run().await.unwrap();
    let metadata = std::fs::metadata(&directory).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    assert_eq!(metadata.uid(), nix::unistd::Uid::current().as_raw());
    let files: Vec<_> = std::fs::read_dir(&directory).unwrap().map(|entry| entry.unwrap().path()).collect();
    assert_eq!(files.len(), 1, "one durable batch and no temporary leftovers");
    let capture = &files[0];
    assert_eq!(capture.extension().unwrap(), "json");
    uuid::Uuid::parse_str(capture.file_stem().unwrap().to_str().unwrap()).unwrap();
    let metadata = std::fs::metadata(capture).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(metadata.uid(), nix::unistd::Uid::current().as_raw());
    let captured: serde_json::Value = serde_json::from_slice(&std::fs::read(capture).unwrap()).unwrap();
    assert_eq!(captured["capture_file"], capture.to_str().unwrap());
    assert_eq!(captured["pending_count"], 0);
    assert_eq!(captured["counts"], serde_json::json!([]));
    let through = captured["through_sequence"].as_u64().expect("captured recipient prefix");
    let instruction = captured["messages"].as_array().unwrap().iter()
        .find(|message| message["text"] == "Retained instruction")
        .expect("explicit instruction remains durably captured");
    assert_eq!(instruction["kind"], "instruction");
    assert_eq!(instruction["stage"], "read");
    let TestCommand::Run(bare) = TestCli::try_parse_from([
        "test", "run", "context",
    ]).unwrap().command else { panic!("expected run") };
    let registered = Context::open(&bare.common, true).await.unwrap();
    let cockpit_core::orchestration::Actor::Agent(caller) = registered.actor.as_ref().unwrap() else {
        panic!("ordinary inbox call must retain credential authority")
    };
    assert!(caller.conversation.is_none());
    let (run, role) = registered.service.job_role(caller).unwrap();
    assert_eq!(role, cockpit_core::orchestration::JobRole::Root);
    assert_eq!(run.stage, cockpit_protocol::orchestration::RunStage::Active);
    assert!(registered.snapshot(None).await.unwrap().messages.iter()
        .filter(|message| message.to_run_id == run_id && message.seq <= through)
        .all(|message| message.stage == cockpit_protocol::orchestration::DeliveryStage::Acked));
    let before_done = registered.snapshot(None).await.unwrap();
    let TestCommand::Run(done) = TestCli::try_parse_from([
        "test", "run", "done", "--failed", "--summary", "root batch",
    ]).unwrap().command else { panic!("expected run") };
    assert_eq!(done.run().await.unwrap_err().code, "actor_forbidden");
    let after_done = registered.snapshot(None).await.unwrap();
    assert_eq!(before_done.revision, after_done.revision, "root done fails before mutation");
    let (still_running, _) = registered.service.job_role(caller).unwrap();
    assert_eq!(still_running.stage, cockpit_protocol::orchestration::RunStage::Active);
    assert_eq!(serde_json::to_value(still_running).unwrap(), serde_json::to_value(run).unwrap(),
        "finishing a root batch must not terminate or create a Result for the liaison");
    let TestCommand::Run(args) = TestCli::try_parse_from([
        "test", "run", "context", "--harness", "other", "--session-id", "actual/session",
    ]).unwrap().command else { panic!("expected run") };
    let first = Context::open(&args.common, true).await.unwrap();
    let before = first.snapshot(None).await.unwrap();
    let second = Context::open(&args.common, true).await.unwrap();
    let after = second.snapshot(None).await.unwrap();
    assert_eq!(before.revision, after.revision, "matching context must not re-register");
    let cockpit_core::orchestration::Actor::Agent(caller) = second.actor.as_ref().unwrap() else { panic!("expected agent") };
    assert_eq!(caller.auth.credential_id, "00000000-0000-4000-8000-000000000001");
    assert_eq!(caller.assignment_generation, caller.auth.assignment_generation);
    assert!(caller.process.is_none());
    assert!(caller.actual_agent_kind.is_none());
    assert_eq!(caller.conversation.as_ref().unwrap().session_id, "actual/session");
    let mut stale = caller.clone();
    stale.assignment_generation = Some(7);
    assert_eq!(second.service.job_role(&stale).unwrap_err().code, "job_unauthorized",
        "caller cannot replace its credential's pinned generation");
    let response = Context::register(&args.common, uuid::Uuid::new_v4().to_string()).unwrap();
    assert!(matches!(response.result,
        cockpit_protocol::orchestration::OrchestrationActionResult::RunRegistered { duplicate: true, .. }));
    let TestCommand::Run(wrong) = TestCli::try_parse_from([
        "test", "run", "context", "--harness", "other", "--session-id", "fork/session",
    ]).unwrap().command else { panic!("expected run") };
    let conflict = match Context::open(&wrong.common, true).await {
        Ok(_) => panic!("another conversation must not rebind a registered capability"),
        Err(error) => error,
    };
    assert_eq!(conflict.code, "job_unauthorized");
}

#[tokio::test]
#[ignore = "private CLI environment child, launched after discarded inbox stdout"]
async fn neutral_cli_recovery_context_child() {
    use clap::Parser;
    use super::{TestCli, TestCommand};
    use super::super::context::Context;
    let TestCommand::Run(bare) = TestCli::try_parse_from([
        "test", "run", "context", "--json",
    ]).unwrap().command else { panic!("expected run") };
    let TestCommand::Run(recovery) = TestCli::try_parse_from([
        "test", "run", "context", "--json",
    ]).unwrap().command else { panic!("expected run") };
    println!();
    recovery.run().await.unwrap();
    let credential = std::env::var_os("COCKPIT_JOB_CREDENTIAL_FILE").unwrap();
    std::fs::write(&credential, b"PRIVATE invalid credential").unwrap();
    let error = match Context::open(&bare.common, true).await {
        Ok(_) => panic!("invalid credential must not fall back to operator authority"),
        Err(error) => error,
    };
    assert_eq!(error.code, "job_credential_invalid");
    assert!(!error.message.contains("PRIVATE"));
}

#[test]
#[ignore = "private CLI environment child, launched with stale operator ambient metadata"]
fn explicit_operator_cli_environment_child() {
    use clap::Parser;
    use super::{TestCli, TestCommand};
    use super::super::context::Context;
    use cockpit_core::orchestration::Actor;
    use cockpit_protocol::orchestration::OperatorOrigin;
    // Drive each CLI invocation independently, as the binary does. Embedding
    // both TaskArgs and RunArgs futures in one async-test frame compounds their
    // large command enums on the libtest worker stack.
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let root = "00000000-0000-4000-8000-000000000003";
    let task = uuid::Uuid::new_v4().to_string();
    let TestCommand::Run(agent) = TestCli::try_parse_from([
        "test", "run", "list",
    ]).unwrap().command else { panic!("expected run") };
    let error = match runtime.block_on(Context::open(&agent.common, false)) {
        Ok(_) => panic!("stale ambient credentials must still fence normal agent mode"),
        Err(error) => error,
    };
    assert!(matches!(error.code.as_str(), "job_credential_invalid" | "unsafe_path"));
    assert!(error.message.contains("--operator"));
    let TestCommand::Task(create) = TestCli::try_parse_from([
        "test", "task", "--operator", "--root", root, "create",
        "--task-id", &task, "--title", "Operator-created task",
    ]).unwrap().command else { panic!("expected task") };
    let context = runtime.block_on(Context::open(&create.common, true)).unwrap();
    assert!(matches!(context.actor, Some(Actor::Operator(OperatorOrigin::Native))));
    assert_eq!(context.session, "physical");
    runtime.block_on(Box::pin(create.run())).unwrap();
    let snapshot = runtime.block_on(context.snapshot(Some(root.into()))).unwrap();
    let revision = snapshot.board.unwrap().tasks.into_iter()
        .find(|view| view.task.task_id == task).unwrap().task.task_revision;
    let TestCommand::Task(update) = TestCli::try_parse_from([
        "test", "task", "update", &task, "--operator", "--root", root,
        "--revision", &revision, "--title", "Operator-updated task",
    ]).unwrap().command else { panic!("expected task") };
    runtime.block_on(Box::pin(update.run())).unwrap();
    let snapshot = runtime.block_on(context.snapshot(Some(root.into()))).unwrap();
    assert_eq!(snapshot.board.unwrap().tasks.into_iter()
        .find(|view| view.task.task_id == task).unwrap().task.title, "Operator-updated task");
    for argv in [
        vec!["test", "task", "--operator", "--root", root, "schedule", &task, "--space", "explicit-space"],
        vec!["test", "task", "--operator", "--root", root, "schedule-cancel", &task],
    ] {
        let TestCommand::Task(args) = TestCli::try_parse_from(argv).unwrap().command else { panic!("expected task") };
        runtime.block_on(Box::pin(args.run())).unwrap();
    }
    let cleanup_file = std::path::Path::new(&context.configuration.state_root)
        .join(format!("operator-cleanup-{task}.json"));
    std::fs::write(&cleanup_file, serde_json::to_vec(&serde_json::json!({
        "operation_id": uuid::Uuid::new_v4().to_string(), "root_id": root, "run_id": root,
    })).unwrap()).unwrap();
    for selected_root in [root, "different-root"] {
        let TestCommand::Run(cleanup) = TestCli::try_parse_from([
            "test", "run", "--operator", "--root", selected_root, "cleanup-finish",
            "--request-file", cleanup_file.to_str().unwrap(),
        ]).unwrap().command else { panic!("expected run") };
        let result = runtime.block_on(Box::pin(cleanup.run()));
        if selected_root == root { result.unwrap(); }
        else { assert_eq!(result.unwrap_err().code, "actor_forbidden"); }
    }
    std::fs::remove_file(cleanup_file).unwrap();
    let TestCommand::Task(foreign) = TestCli::try_parse_from([
        "test", "task", "--operator", "--root", root, "--herdr-session", "foreign",
        "create", "--task-id", &uuid::Uuid::new_v4().to_string(), "--title", "Wrong session",
    ]).unwrap().command else { panic!("expected task") };
    assert_eq!(runtime.block_on(Box::pin(foreign.run())).unwrap_err().code, "session_mismatch");
    let TestCommand::Task(no_root) = TestCli::try_parse_from([
        "test", "task", "--operator", "create", "--title", "No implicit write root",
    ]).unwrap().command else { panic!("expected task") };
    assert_eq!(runtime.block_on(Box::pin(no_root.run())).unwrap_err().code, "orchestration_usage");
}

#[test]
fn no_extension_context_registration_uses_private_capability_without_live_herdr() {
    use std::os::unix::fs::PermissionsExt;
    use sha2::{Digest, Sha256};
    use base64::Engine;
    let root = std::env::temp_dir().join(format!("cockpit-cli-neutral-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("state/orchestration")).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir(root.join("catalog")).unwrap();
    let config = root.join("config.toml");
    let directory = root.to_str().unwrap();
    std::fs::write(&config, format!(
        "repository_roots = ['{directory}/catalog']\nstate_root = '{directory}/state'\ncache_root = '{directory}/cache'\nworktree_root = '{directory}/worktrees'\nlibrary_root = '{directory}/library'\nnotes_root = '{directory}/notes'\n"
    )).unwrap();
    let run_id = "00000000-0000-4000-8000-000000000003";
    let credential_id = "00000000-0000-4000-8000-000000000001";
    let attachment_id = "00000000-0000-4000-8000-000000000002";
    let launch = cockpit_protocol::projects::OrchestrationConfiguration {
        harness_kind: "other".into(),
        command: "other".into(),
        ..Default::default()
    };
    let run = serde_json::json!({
        "session_id":"physical", "brief":"", "run_id":run_id, "kind":"supervisor",
        "label":"neutral", "root_id":run_id, "attempt":1, "assignment_generation":null,
        "launch_config":launch, "stage":"starting", "annotations":[],
        "attachment":{"attachment_id":attachment_id, "epoch":1, "state":"connecting",
            "launch_attempt":1, "updated_at":"2026-10-12T00:00:00Z"},
        "created_at":"2026-10-12T00:00:00Z", "updated_at":"2026-10-12T00:00:00Z"
    });
    use cockpit_protocol::orchestration::{DeliveryStage, MessageKind, Report, ReportKind};
    let mut progress = super::message(run_id, 1, MessageKind::Report, DeliveryStage::Stored, "Hidden progress");
    progress.report = Some(Report {
        message_id: progress.message_id.clone(), kind: ReportKind::Progress, outcome: None,
        summary: "Hidden progress".into(), plan: None, source_output: None, at: progress.created_at.clone(),
    });
    let instruction = super::message(run_id, 2, MessageKind::Instruction, DeliveryStage::Stored, "Retained instruction");
    let state = serde_json::json!({
        "schema":4, "revision":1, "runs":[run], "messages":[progress, instruction], "task_ownership":[], "inbox_hints":[],
        "task_intents":[], "assignment_intents":[], "scope_intents":[], "checkout_claims":[],
        "resource_requests":[], "resource_sequence":0, "owned_resources":[],
        "operations":[], "credentials":[{
            "credential_id":credential_id,"run_id":run_id,"root_id":run_id,"role":"root",
            "assignment_generation":null,"attachment_id":attachment_id,"attachment_epoch":1,
            "secret_digest":format!("{:x}", Sha256::digest([7_u8;32])),
            "state":"active","issued_at":"2026-10-12T00:00:00Z"
        }]
    });
    let credentials = root.join("credentials");
    std::fs::create_dir(&credentials).unwrap();
    std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o700)).unwrap();
    let credential = credentials.join(format!("{attachment_id}-1-{credential_id}.json"));
    std::fs::write(&credential, serde_json::to_vec(&serde_json::json!({
        "schema":2,"credential_id":credential_id,"run_id":run_id,"root_id":run_id,"role":"root",
        "assignment_generation":null,"attachment_id":attachment_id,"attachment_epoch":1,
        "secret":base64::engine::general_purpose::STANDARD.encode([7_u8;32])
    })).unwrap()).unwrap();
    std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(root.join("state/orchestration/state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
    let binding = serde_json::to_vec(&(run_id, attachment_id, 1_u64, credential_id)).unwrap();
    let capture_directory = credentials.join(format!("inbox-capture-{:x}", Sha256::digest(binding)));
    assert!(!capture_directory.exists());
    let child = |name: &str| {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", name, "--ignored", "--nocapture"])
            .env_clear()
            .env("HOME", &root)
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("COCKPIT_CONFIG_PATH", &config)
            .env("COCKPIT_JOB_CREDENTIAL_FILE", &credential)
            .env("COCKPIT_RUN_ID", run_id)
            .env("COCKPIT_HERDR_SESSION", "physical")
            .env("COCKPIT_HERDR_EXECUTABLE", "/definitely/absent/herdr")
            .env("PI_SESSION_FILE", "/definitely/absent/session-file");
        command
    };
    // Lose all first-call stdout: recovery must not depend on its returned path or bodies.
    let output = child("cli_orchestration::tests::identity::neutral_cli_environment_child")
        .stdout(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let files: Vec<_> = std::fs::read_dir(&capture_directory).unwrap()
        .map(|entry| entry.unwrap().path()).collect();
    assert_eq!(files.len(), 1);
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!(stored["capture_file"], files[0].to_str().unwrap());
    let instruction = stored["messages"].as_array().unwrap().iter()
        .find(|message| message["text"] == "Retained instruction")
        .expect("explicit instruction survives discarded stdout");
    assert_eq!(instruction["kind"], "instruction");
    assert_eq!(instruction["stage"], "read", "capture precedes ACK");
    let through = stored["through_sequence"].as_u64().expect("captured recipient prefix");
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("state/orchestration/state.json")).unwrap()).unwrap();
    assert!(state["messages"].as_array().unwrap().iter()
        .filter(|message| message["to_run_id"] == run_id && message["seq"].as_u64().unwrap() <= through)
        .all(|message| message["stage"] == "acked"));
    let recovery = child("cli_orchestration::tests::identity::neutral_cli_recovery_context_child")
        .output().unwrap();
    assert!(recovery.status.success(), "{}", String::from_utf8_lossy(&recovery.stderr));
    let operator = child("cli_orchestration::tests::identity::explicit_operator_cli_environment_child")
        .env("HERDR_ENV", "1")
        .env("COCKPIT_RUN_ID", "stale-run")
        .env("COCKPIT_ATTACHMENT_ID", "stale-attachment")
        .env("COCKPIT_ATTACHMENT_EPOCH", "999")
        .env("COCKPIT_HARNESS", "stale-harness")
        .env("COCKPIT_SESSION_ID", "stale-conversation")
        .output().unwrap();
    assert!(operator.status.success(), "{}", String::from_utf8_lossy(&operator.stderr));
    let missing = child("cli_orchestration::tests::identity::explicit_operator_cli_environment_child")
        .env("HERDR_ENV", "1")
        .env("COCKPIT_JOB_CREDENTIAL_FILE", root.join("missing-credential"))
        .env("COCKPIT_RUN_ID", "stale-run")
        .env("COCKPIT_ATTACHMENT_ID", "stale-attachment")
        .env("COCKPIT_HARNESS", "stale-harness")
        .env("COCKPIT_SESSION_ID", "stale-conversation")
        .output().unwrap();
    assert!(missing.status.success(), "{}", String::from_utf8_lossy(&missing.stderr));
    let stdout = String::from_utf8_lossy(&recovery.stdout);
    let context = stdout.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value.get("inbox_capture_directory").is_some()).expect("subsequent run context pointer");
    assert_eq!(context["inbox_capture_directory"], capture_directory.to_str().unwrap());
    assert_eq!(context["run_id"], run_id);
    let recovered = std::path::Path::new(context["inbox_capture_directory"].as_str().unwrap())
        .join(files[0].file_name().unwrap());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(recovered).unwrap()).unwrap(), stored);
    let worker = serde_json::json!({
        "schema":2,"credential_id":credential_id,"run_id":run_id,
        "root_id":"00000000-0000-4000-8000-000000000004","role":"worker",
        "assignment_generation":7,"attachment_id":attachment_id,"attachment_epoch":1,
        "secret":base64::engine::general_purpose::STANDARD.encode([7_u8;32])
    });
    std::fs::write(&credential, serde_json::to_vec(&worker).unwrap()).unwrap();
    assert_eq!(cockpit_core::orchestration::read_job_auth(&credential).unwrap().assignment_generation, Some(7));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "private CLI environment child, launched by the artifact Result boundary test"]
async fn artifact_result_cli_child() {
    use clap::Parser;
    use super::{TestCli, TestCommand};
    use super::super::context::Context;
    use cockpit_protocol::orchestration::{CheckStatus, SourceOutput};
    let parse = |flags: &[&str]| {
        let TestCommand::Run(args) = TestCli::try_parse_from(
            ["test", "run"].into_iter().chain(flags.iter().copied()),
        ).unwrap().command else { panic!("expected run") };
        args
    };
    let initial = parse(&["context", "--json"]);
    let context = Context::open(&initial.common, true).await.unwrap();
    let snapshot = context.snapshot(None).await.unwrap();
    let task = &snapshot.board.as_ref().unwrap().tasks[0].task;
    if std::env::var("COCKPIT_TEST_RESULT_ROLE").unwrap() == "root" {
        let worker = snapshot.runs.iter().find(|run| run.task_id.is_some()).unwrap();
        assert!(matches!(&worker.source_output, Some(SourceOutput::NonCode {
            task_revision, result_message_id, ..
        }) if task_revision == &task.task_revision && result_message_id == "research-result"));
        parse(&["accept", &worker.run_id, "--task-revision", &task.task_revision, "--json"])
            .run().await.unwrap();
        let accepted = context.snapshot(None).await.unwrap();
        assert!(accepted.board.unwrap().tasks[0].task.checked);
        return;
    }
    let run = context.own_run(&snapshot).unwrap();
    if run.checkout.as_ref().unwrap().git.is_some() {
        let output = context.source_output("code-result", Vec::new(), Vec::new()).await.unwrap();
        assert!(matches!(output, SourceOutput::Code {
            result_message_id, task_revision, base_commit, head_commit, commits, ..
        } if result_message_id == "code-result" && task_revision == task.task_revision
            && base_commit == head_commit && commits.is_empty()));
    } else {
        assert_eq!(context.source_output("missing-artifact", Vec::new(), Vec::new())
            .await.unwrap_err().code, "orchestration_usage");
    }
    let artifact = std::env::var("COCKPIT_TEST_RESULT_ARTIFACT").unwrap();
    let malformed = parse(&["done", "--message-id", "invalid-check", "--summary", "Research",
        "--artifact", &artifact, "--check", "research::read report::unknown"]);
    assert_eq!(malformed.run().await.unwrap_err().code, "orchestration_usage");
    let invalid = parse(&["done", "--message-id", "invalid-artifact", "--summary", "Research",
        "--artifact", " "]);
    assert_eq!(invalid.run().await.unwrap_err().code, "invalid_source_output");
    assert_eq!(context.snapshot(None).await.unwrap().revision, snapshot.revision);
    parse(&["done", "--message-id", "research-result", "--summary", "Read-only repository analysis",
        "--artifact", &artifact, "--check", "research::read analysis report::passed", "--json"])
        .run().await.unwrap();
    let reported = context.snapshot(None).await.unwrap();
    let worker = context.own_run(&reported).unwrap();
    assert!(matches!(&worker.source_output, Some(SourceOutput::NonCode {
        result_message_id, task_revision, artifacts, focused_checks,
    }) if result_message_id == "research-result" && task_revision == &task.task_revision
        && artifacts.as_slice() == std::slice::from_ref(&artifact) && focused_checks.len() == 1
        && focused_checks[0].outcome == CheckStatus::Passed
        && focused_checks[0].contract == "research"
        && focused_checks[0].command_or_scenario == "read analysis report"));
}

#[tokio::test]
async fn explicit_artifact_result_from_git_worktree_and_directory_is_root_acceptable() {
    use std::os::unix::fs::PermissionsExt;
    use sha2::{Digest, Sha256};
    use base64::Engine;
    use serde_json::json;
    for git_checkout in [true, false] {
        let root = std::env::temp_dir().join(format!("cockpit-cli-artifact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("state/orchestration/tasks")).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(root.join("catalog")).unwrap();
        let checkout_path = root.join("research");
        if git_checkout {
            let repository = root.join("catalog/repository");
            let git = |directory: &std::path::Path, args: &[&str]| {
                let output = std::process::Command::new("git").current_dir(directory)
                    .args(args).output().unwrap();
                assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            };
            git(&root, &["init", "--initial-branch=main", repository.to_str().unwrap()]);
            git(&repository, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "--allow-empty", "-m", "Fixture base"]);
            git(&repository, &["worktree", "add", "-b", "research", checkout_path.to_str().unwrap()]);
        } else {
            std::fs::create_dir(&checkout_path).unwrap();
        }
        let (project, checkout) = cockpit_core::orchestration::project_identity::resolve_directory(
            &checkout_path, None, None,
        ).await.unwrap();
        let base_commit = if git_checkout {
            Some(cockpit_core::space_git::git_line(&checkout_path,
                &["rev-parse", "--verify", "HEAD^{commit}"]).await.unwrap())
        } else { None };
        let config = root.join("config.toml");
        let directory = root.to_str().unwrap();
        std::fs::write(&config, format!(
            "repository_roots = ['{directory}/catalog']\nstate_root = '{directory}/state'\ncache_root = '{directory}/cache'\nworktree_root = '{directory}/worktrees'\nlibrary_root = '{directory}/library'\nnotes_root = '{directory}/notes'\n"
        )).unwrap();
        let root_id = uuid::Uuid::new_v4().to_string();
        let worker_id = uuid::Uuid::new_v4().to_string();
        let task_id = uuid::Uuid::new_v4().to_string();
        let claim_id = uuid::Uuid::new_v4().to_string();
        std::fs::write(root.join(format!("state/orchestration/tasks/{root_id}.md")),
            format!("# Tasks\n\n- [ ] Read-only repository analysis <!-- cockpit-task: {task_id} -->\n")).unwrap();
        let artifact = root.join("analysis.txt");
        std::fs::write(&artifact, "Actual analysis of the repository; no code changes required.\n").unwrap();
        let retained = checkout_path.join("user-draft.txt");
        std::fs::write(&retained, "Pre-existing unrelated draft\n").unwrap();
        let credentials_dir = root.join("credentials");
        std::fs::create_dir(&credentials_dir).unwrap();
        std::fs::set_permissions(&credentials_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut runs = Vec::new();
        let mut credentials = Vec::new();
        let mut capabilities = Vec::new();
        for (run_id, role, generation, kind, stage) in [
            (&root_id, "root", None, "supervisor", "active"),
            (&worker_id, "worker", Some(1_u64), "worker", "working"),
        ] {
            let attachment_id = uuid::Uuid::new_v4().to_string();
            let credential_id = uuid::Uuid::new_v4().to_string();
            let mut run = json!({
                "session_id":"physical", "brief":"Read-only repository analysis", "run_id":run_id,
                "kind":kind, "label":role, "root_id":root_id, "attempt":1,
                "assignment_generation":generation, "launch_config":{"harness_kind":"other","command":"other"},
                "stage":stage, "annotations":[], "created_at":"2026-10-14T00:00:00Z",
                "updated_at":"2026-10-14T00:00:00Z",
                "attachment":{"attachment_id":attachment_id,"epoch":1,"state":"connected",
                    "launch_attempt":1,"updated_at":"2026-10-14T00:00:00Z"}
            });
            if role == "worker" {
                run["parent_run_id"] = json!(root_id);
                run["task_id"] = json!(task_id);
                run["project"] = json!(project);
                run["checkout"] = json!(checkout);
                run["checkout_claim_id"] = json!(claim_id);
                run["setup"] = json!({"checkout_path":checkout.path,"base_commit":base_commit,
                    "effects":[],"warnings":[]});
            }
            credentials.push(json!({"credential_id":credential_id,"run_id":run_id,"root_id":root_id,
                "role":role,"assignment_generation":generation,"attachment_id":attachment_id,
                "attachment_epoch":1,"secret_digest":format!("{:x}", Sha256::digest([7_u8;32])),
                "state":"active","issued_at":"2026-10-14T00:00:00Z"}));
            let credential = credentials_dir.join(format!("{attachment_id}-1-{credential_id}.json"));
            std::fs::write(&credential, serde_json::to_vec(&json!({
                "schema":2,"credential_id":credential_id,"run_id":run_id,"root_id":root_id,"role":role,
                "assignment_generation":generation,"attachment_id":attachment_id,"attachment_epoch":1,
                "secret":base64::engine::general_purpose::STANDARD.encode([7_u8;32])
            })).unwrap()).unwrap();
            std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
            capabilities.push(credential);
            runs.push(run);
        }
        let state = json!({"schema":4,"revision":1,"runs":runs,"credentials":credentials,
            "messages":[],"inbox_hints":[],"task_intents":[],"assignment_intents":[],
            "scope_intents":[],"operations":[],
            "task_ownership":[{"root_id":root_id,"task_id":task_id,"generation":1,"schedule":null,
                "owner":{"run_id":worker_id,"conversation":null,"claim_operation_id":uuid::Uuid::new_v4().to_string()}}],
            "checkout_claims":[{"claim_id":claim_id,"checkout":checkout,"revision":1,"state":"held",
                "holder":{"run_id":worker_id,"attachment_id":runs[1]["attachment"]["attachment_id"],
                    "attachment_epoch":1},"transition_operation_id":null}]});
        let state_path = root.join("state/orchestration/state.json");
        std::fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
        for (run_id, role, credential) in [
            (&worker_id, "worker", &capabilities[1]), (&root_id, "root", &capabilities[0]),
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "cli_orchestration::tests::identity::artifact_result_cli_child",
                    "--ignored", "--nocapture"])
                .env_clear().env("PATH", std::env::var_os("PATH").unwrap())
                .env("HOME", &root).env("COCKPIT_CONFIG_PATH", &config)
                .env("COCKPIT_JOB_CREDENTIAL_FILE", credential).env("COCKPIT_RUN_ID", run_id)
                .env("COCKPIT_HERDR_SESSION", "physical")
                .env("COCKPIT_HERDR_EXECUTABLE", "/definitely/absent/herdr")
                .env("COCKPIT_TEST_RESULT_ROLE", role).env("COCKPIT_TEST_RESULT_ARTIFACT", &artifact)
                .output().unwrap();
            assert!(output.status.success(), "stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
        let accepted: serde_json::Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
        assert_eq!(accepted["runs"][1]["stage"], "closed");
        assert_eq!(accepted["runs"][1]["close_reason"], "accepted");
        assert_eq!(accepted["runs"][1]["source_output"]["kind"], "non_code");
        assert_eq!(accepted["runs"][1]["integration"]["state"], "not_applicable");
        assert_eq!(accepted["runs"][1]["integration"]["result_message_id"], "research-result");
        assert_eq!(std::fs::read_to_string(&retained).unwrap(), "Pre-existing unrelated draft\n");
        assert_eq!(std::fs::read_to_string(&artifact).unwrap(),
            "Actual analysis of the repository; no code changes required.\n");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
#[ignore = "private CLI environment child, launched by the scoped snapshot boundary test"]
async fn scoped_snapshot_cli_environment_child() {
    use clap::Parser;
    use super::{TestCli, TestCommand};
    let command = std::env::var("COCKPIT_TEST_SNAPSHOT_COMMAND").unwrap();
    let sibling = std::env::var("COCKPIT_TEST_SNAPSHOT_SIBLING").unwrap();
    let foreign = std::env::var("COCKPIT_TEST_SNAPSHOT_FOREIGN").unwrap();
    let foreign_root = std::env::var("COCKPIT_TEST_SNAPSHOT_FOREIGN_ROOT").unwrap();
    let flags = match command.as_str() {
        "list" => vec!["list", "--json"],
        "operator" => vec!["--operator", "list", "--json"],
        "self" => vec!["show", "--self", "--json"],
        "sibling" => vec!["show", sibling.as_str(), "--json"],
        "foreign" => vec!["show", foreign.as_str(), "--json"],
        "foreign-root" => vec!["--root", foreign_root.as_str(), "list", "--json"],
        _ => panic!("unknown private snapshot command"),
    };
    let TestCommand::Run(args) = TestCli::try_parse_from(
        ["test", "run"].into_iter().chain(flags),
    ).unwrap().command else { panic!("expected run") };
    let result = args.run().await;
    match command.as_str() {
        "foreign" => assert_eq!(result.unwrap_err().code, "run_not_found"),
        "foreign-root" => assert_eq!(result.unwrap_err().code, "actor_forbidden"),
        _ => result.unwrap(),
    }
}

#[tokio::test]
async fn credentialed_cli_reads_are_root_scoped_private_and_inert() {
    use base64::Engine;
    use cockpit_protocol::orchestration::{DeliveryStage, MessageKind};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;

    let directory = std::env::temp_dir().join(format!("cockpit-cli-scoped-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(directory.join("state/orchestration/tasks")).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir(directory.join("catalog")).unwrap();
    let config = directory.join("config.toml");
    let path = directory.to_str().unwrap();
    std::fs::write(&config, format!(
        "repository_roots = ['{path}/catalog']\nstate_root = '{path}/state'\ncache_root = '{path}/cache'\nworktree_root = '{path}/worktrees'\nlibrary_root = '{path}/library'\nnotes_root = '{path}/notes'\n"
    )).unwrap();
    let credentials_dir = directory.join("credentials");
    std::fs::create_dir(&credentials_dir).unwrap();
    std::fs::set_permissions(&credentials_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root_id = uuid::Uuid::new_v4().to_string();
    let worker_id = uuid::Uuid::new_v4().to_string();
    let sibling_id = uuid::Uuid::new_v4().to_string();
    let foreign_root_id = uuid::Uuid::new_v4().to_string();
    let foreign_worker_id = uuid::Uuid::new_v4().to_string();
    let task_id = uuid::Uuid::new_v4().to_string();
    let sibling_task_id = uuid::Uuid::new_v4().to_string();
    let foreign_task_id = uuid::Uuid::new_v4().to_string();
    let task_path = directory.join(format!("state/orchestration/tasks/{root_id}.md"));
    let foreign_task_path = directory.join(format!("state/orchestration/tasks/{foreign_root_id}.md"));
    std::fs::write(&task_path, format!(
        "# Tasks\n\n- [ ] Own task <!-- cockpit-task: {task_id} -->\n- [ ] Sibling task <!-- cockpit-task: {sibling_task_id} -->\n"
    )).unwrap();
    std::fs::write(&foreign_task_path, format!(
        "# Tasks\n\n- [ ] Foreign task <!-- cockpit-task: {foreign_task_id} -->\n"
    )).unwrap();
    let mut runs = Vec::new();
    let mut credentials = Vec::new();
    let mut capabilities = Vec::new();
    let mut claims = Vec::new();
    let mut ownership = Vec::new();
    for (run_id, run_root, task) in [
        (&root_id, &root_id, None),
        (&worker_id, &root_id, Some(&task_id)),
        (&sibling_id, &root_id, Some(&sibling_task_id)),
        (&foreign_root_id, &foreign_root_id, None),
        (&foreign_worker_id, &foreign_root_id, Some(&foreign_task_id)),
    ] {
        let attachment_id = uuid::Uuid::new_v4().to_string();
        let role = if task.is_some() { "worker" } else { "root" };
        let generation = task.map(|_| 1_u64);
        let mut run = json!({
            "session_id":"physical", "brief":"Snapshot fixture", "run_id":run_id,
            "kind":if task.is_some() { "worker" } else { "supervisor" },
            "label":role, "root_id":run_root, "attempt":1, "assignment_generation":generation,
            "launch_config":{"harness_kind":"other","command":"other"},
            "stage":if task.is_some() { "working" } else { "active" },
            "annotations":[], "created_at":"2026-10-17T00:00:00Z", "updated_at":"2026-10-17T00:00:00Z",
            "attachment":{"attachment_id":attachment_id,"epoch":1,"state":"connected",
                "launch_attempt":1,"updated_at":"2026-10-17T00:00:00Z"},
            "dispatch":{"agent_started":true,"step":"launched","launch_attempt":1,
                "updated_at":"2026-10-17T00:00:00Z",
                "latch":{"latch":format!("/private/launch-SECRET-NONCE-{run_id}.fifo"),"nonce":format!("SECRET-NONCE-{run_id}"),
                    "latch_device":"1","latch_inode":"2"}}
        });
        if let Some(task) = task {
            let checkout_path = directory.join(format!("checkout-{run_id}"));
            std::fs::create_dir(&checkout_path).unwrap();
            let (project, checkout) = cockpit_core::orchestration::project_identity::resolve_directory(
                &checkout_path, None, None,
            ).await.unwrap();
            let claim_id = uuid::Uuid::new_v4().to_string();
            run["parent_run_id"] = json!(run_root);
            run["task_id"] = json!(task);
            run["project"] = json!(project);
            run["checkout"] = json!(checkout);
            run["checkout_claim_id"] = json!(claim_id);
            claims.push(json!({"claim_id":claim_id,"checkout":checkout,"revision":1,"state":"held",
                "holder":{"run_id":run_id,"attachment_id":attachment_id,"attachment_epoch":1},
                "transition_operation_id":null}));
            ownership.push(json!({"root_id":run_root,"task_id":task,"generation":1,"schedule":null,
                "owner":{"run_id":run_id,"conversation":null,
                    "claim_operation_id":uuid::Uuid::new_v4().to_string()}}));
        }
        if run_id == &root_id || run_id == &worker_id {
            let credential_id = uuid::Uuid::new_v4().to_string();
            credentials.push(json!({"credential_id":credential_id,"run_id":run_id,"root_id":run_root,
                "role":role,"assignment_generation":generation,"attachment_id":attachment_id,
                "attachment_epoch":1,"secret_digest":format!("{:x}", Sha256::digest([7_u8;32])),
                "state":"active","issued_at":"2026-10-17T00:00:00Z"}));
            let credential = credentials_dir.join(format!("{attachment_id}-1-{credential_id}.json"));
            std::fs::write(&credential, serde_json::to_vec(&json!({
                "schema":2,"credential_id":credential_id,"run_id":run_id,"root_id":run_root,"role":role,
                "assignment_generation":generation,"attachment_id":attachment_id,"attachment_epoch":1,
                "secret":base64::engine::general_purpose::STANDARD.encode([7_u8;32])
            })).unwrap()).unwrap();
            std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
            capabilities.push(credential);
        }
        runs.push(run);
    }
    let messages = [
        super::message(&worker_id, 1, MessageKind::Instruction, DeliveryStage::Stored, "OWN-INBOX-MARKER"),
        super::message(&sibling_id, 2, MessageKind::Instruction, DeliveryStage::Stored, "PRIVATE-SIBLING-INBOX"),
        super::message(&root_id, 3, MessageKind::Observation, DeliveryStage::Stored, "PRIVATE-ROOT-INBOX"),
        super::message(&foreign_worker_id, 4, MessageKind::Instruction, DeliveryStage::Stored, "PRIVATE-FOREIGN-INBOX"),
    ];
    let state = json!({"schema":4,"revision":1,"runs":runs,"credentials":credentials,
        "messages":messages,"inbox_hints":[],"task_intents":[],"assignment_intents":[],
        "scope_intents":[],"operations":[],"task_ownership":ownership,"checkout_claims":claims});
    let state_path = directory.join("state/orchestration/state.json");
    std::fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    let state_before = std::fs::read(&state_path).unwrap();
    let tasks_before = std::fs::read(&task_path).unwrap();
    let foreign_tasks_before = std::fs::read(&foreign_task_path).unwrap();
    for (role, run_id, credential) in [
        ("worker", &worker_id, &capabilities[1]),
        ("root", &root_id, &capabilities[0]),
    ] {
        for command in ["list", "self", "sibling", "foreign", "foreign-root", "operator"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "cli_orchestration::tests::identity::scoped_snapshot_cli_environment_child",
                    "--ignored", "--nocapture"])
                .env_clear()
                .env("HOME", &directory)
                .env("XDG_CACHE_HOME", directory.join("cache"))
                .env("XDG_CONFIG_HOME", directory.join("config"))
                .env("XDG_DATA_HOME", directory.join("data"))
                .env("XDG_STATE_HOME", directory.join("state"))
                .env("COCKPIT_CONFIG_PATH", &config)
                .env("COCKPIT_JOB_CREDENTIAL_FILE", credential)
                .env("COCKPIT_RUN_ID", run_id)
                .env("COCKPIT_HERDR_SESSION", "physical")
                .env("COCKPIT_HERDR_EXECUTABLE", "/definitely/absent/herdr")
                .env("COCKPIT_TEST_SNAPSHOT_COMMAND", command)
                .env("COCKPIT_TEST_SNAPSHOT_SIBLING", &sibling_id)
                .env("COCKPIT_TEST_SNAPSHOT_FOREIGN", &foreign_worker_id)
                .env("COCKPIT_TEST_SNAPSHOT_FOREIGN_ROOT", &foreign_root_id)
                .output().unwrap();
            assert!(output.status.success(), "{role} {command}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert_eq!(std::fs::read(&state_path).unwrap(), state_before, "{role} {command} changed state");
            assert_eq!(std::fs::read(&task_path).unwrap(), tasks_before);
            assert_eq!(std::fs::read(&foreign_task_path).unwrap(), foreign_tasks_before);
            if matches!(command, "foreign" | "foreign-root") { continue; }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let value = stdout.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .find(|value| value.is_object()).expect("CLI JSON output");
            let wire = serde_json::to_string(&value).unwrap();
            if command == "operator" {
                assert!(wire.contains(&foreign_root_id));
                assert!(wire.contains(&foreign_worker_id));
                assert!(wire.contains("SECRET-NONCE-"));
                assert!(wire.contains("PRIVATE-SIBLING-INBOX"));
                assert!(wire.contains("PRIVATE-FOREIGN-INBOX"));
                continue;
            }
            for private in [&foreign_root_id, &foreign_worker_id, &foreign_task_id] {
                assert!(!wire.contains(private), "{role} {command} leaked a foreign identity");
            }
            assert!(!wire.contains("SECRET-NONCE-"), "{role} {command} leaked a launch nonce");
            assert!(!wire.contains("PRIVATE-FOREIGN-INBOX"));
            if command == "list" {
                assert_eq!(value["roots"].as_array().unwrap().len(), 1);
                assert_eq!(value["runs"].as_array().unwrap().len(), 3);
                assert!(wire.contains(&root_id));
                assert!(wire.contains(&worker_id));
                assert!(wire.contains(&sibling_id));
                assert!(wire.contains("OWN-INBOX-MARKER"));
                let recipients: Vec<_> = value["messages"].as_array().unwrap().iter()
                    .map(|message| message["to_run_id"].as_str().unwrap()).collect();
                if role == "worker" {
                    assert_eq!(recipients, vec![worker_id.as_str()]);
                    assert!(!wire.contains("PRIVATE-SIBLING-INBOX"));
                    assert!(!wire.contains("PRIVATE-ROOT-INBOX"));
                } else {
                    assert_eq!(recipients.len(), 3);
                    assert!(wire.contains("PRIVATE-SIBLING-INBOX"));
                    assert!(wire.contains("PRIVATE-ROOT-INBOX"));
                }
            } else {
                assert_eq!(value["run_id"].as_str().unwrap(),
                    if command == "self" { run_id.as_str() } else { sibling_id.as_str() });
                assert_eq!(value["dispatch"]["latch"]["nonce"], "[REDACTED]");
                assert!(!wire.contains("PRIVATE-SIBLING-INBOX"));
            }
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
