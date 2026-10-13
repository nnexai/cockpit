use super::super::caller::resolve_conversation;

#[test]
fn explicit_non_omp_identity_needs_no_journal_and_rejects_conflicting_sources() {
    let missing = std::path::Path::new("/definitely/absent/session-file");
    let identity = resolve_conversation(Some("other".into()), Some("opaque/session".into()),
        None, None, Some(missing)).unwrap();
    assert_eq!(identity.harness, "other");
    assert_eq!(identity.session_id, "opaque/session");
    assert!(resolve_conversation(None, Some("id".into()), None, None, None).is_err());
    assert!(resolve_conversation(Some("other".into()), None, None, None, None).is_err());
    for (harness, session) in [("omp", "id"), ("other", "stale")] {
        assert_eq!(resolve_conversation(Some(harness.into()), Some(session.into()),
            Some("other".into()), Some("id".into()), None).unwrap_err().code, "conversation_conflict");
    }
    for invalid in [String::new(), "a".repeat(513), "bad\nidentity".into()] {
        assert!(resolve_conversation(Some("other".into()), Some(invalid), None, None, None).is_err());
    }
}

#[test]
fn omp_header_is_checked_against_explicit_and_environment_identity() {
    let root = std::env::temp_dir().join(format!("cockpit-cli-header-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let file = root.join("unrelated-name.jsonl");
    std::fs::write(&file, b"{\"type\":\"title\"}\n{\"type\":\"session\",\"id\":\"actual\"}\n").unwrap();
    assert_eq!(resolve_conversation(Some("omp".into()), None, None, None, Some(&file)).unwrap().session_id, "actual");
    assert_eq!(resolve_conversation(Some("omp".into()), Some("actual".into()),
        Some("omp".into()), Some("actual".into()), Some(&file)).unwrap().session_id, "actual");
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
        let error = resolve_conversation(Some("omp".into()), Some("actual".into()),
            None, None, Some(path)).unwrap_err();
        assert_eq!(error.code, "session_header_invalid");
        assert!(!error.message.contains("PRIVATE"));
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
    let mut stale = caller.clone();
    stale.assignment_generation = Some(7);
    assert!(second.service.job_role(&stale).is_err(), "caller cannot replace its credential's pinned generation");
    let response = Context::register(&args.common, uuid::Uuid::new_v4().to_string()).unwrap();
    assert!(matches!(response.result,
        cockpit_protocol::orchestration::OrchestrationActionResult::RunRegistered { duplicate: true, .. }));
    let TestCommand::Run(wrong) = TestCli::try_parse_from([
        "test", "run", "context", "--harness", "other", "--session-id", "fork/session",
    ]).unwrap().command else { panic!("expected run") };
    assert!(Context::open(&wrong.common, true).await.is_err());
    let TestCommand::Inbox(wait) = TestCli::try_parse_from([
        "test", "inbox", "wait", "--timeout", "0", "--harness", "other", "--session-id", "actual/session",
    ]).unwrap().command else { panic!("expected inbox") };
    wait.run().await.unwrap();
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
    let state = serde_json::json!({
        "schema":3, "revision":1, "runs":[run], "messages":[], "task_ownership":[], "inbox_hints":[],
        "task_intents":[], "assignment_intents":[], "scope_intents":[], "checkout_claims":[],
        "resource_requests":[], "resource_sequence":0, "owned_resources":[],
        "operations":[], "credentials":[{
            "credential_id":credential_id,"run_id":run_id,"root_id":run_id,"role":"root",
            "assignment_generation":null,"attachment_id":attachment_id,"attachment_epoch":1,
            "secret_digest":format!("{:x}", Sha256::digest([7_u8;32])),
            "state":"active","issued_at":"2026-10-12T00:00:00Z"
        }]
    });
    std::fs::write(root.join("state/orchestration/state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
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
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","cli_orchestration::tests::identity::neutral_cli_environment_child","--ignored","--nocapture"])
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
        .env("PI_SESSION_FILE", "/definitely/absent/session-file")
        .output().unwrap();
    let worker = serde_json::json!({
        "schema":2,"credential_id":credential_id,"run_id":run_id,
        "root_id":"00000000-0000-4000-8000-000000000004","role":"worker",
        "assignment_generation":7,"attachment_id":attachment_id,"attachment_epoch":1,
        "secret":base64::engine::general_purpose::STANDARD.encode([7_u8;32])
    });
    std::fs::write(&credential, serde_json::to_vec(&worker).unwrap()).unwrap();
    assert_eq!(cockpit_core::orchestration::read_job_auth(&credential).unwrap().assignment_generation, Some(7));
    std::fs::remove_dir_all(root).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
