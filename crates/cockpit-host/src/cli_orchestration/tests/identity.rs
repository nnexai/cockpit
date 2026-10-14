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
    let mut launch = cockpit_protocol::projects::OrchestrationConfiguration::default();
    launch.harness_kind = "other".into();
    launch.command = "other".into();
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
