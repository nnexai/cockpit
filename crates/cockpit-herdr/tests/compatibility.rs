use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cockpit_core::HerdrAdapter;
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_protocol::v1::HerdrCompatibility;
#[cfg(unix)]
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn temp_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(unix)]
fn fixture_script(status: &str, schema: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let id = temp_id();
    let path = std::env::temp_dir().join(format!("cockpit-herdr-test-{id}.sh"));
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in *'status server --json') printf '%s' '{}';; *'api schema --json') printf '%s' '{}';; esac\n",
        status.replace('\'', "'\\''"),
        schema.replace('\'', "'\\''")
    );
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
async fn inspect(status: &str, schema: &str) -> HerdrCompatibility {
    let script = fixture_script(status, schema);
    let config = HerdrCliConfig::from_options(Some(script.clone()), None, None).unwrap();
    let result = HerdrCliAdapter::new(config)
        .inspect()
        .await
        .unwrap_or_else(|error| panic!("unexpected inspection failure: {}", error));
    drop(fs::remove_file(script));
    result
}

#[cfg(unix)]
#[tokio::test]
async fn installed_schema_fixture_is_compatible() {
    let schema = include_str!("fixtures/herdr-0.8.2-protocol-20-schema-1.json");
    let status = r#"{"version":"0.8.2","protocol":20}"#;
    assert!(
        matches!(inspect(status, schema).await, HerdrCompatibility::Compatible { identity } if identity.version == "0.8.2" && identity.protocol == 20 && identity.schema_version == 1)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn autostart_launches_a_missing_server_before_inspection() {
    use std::os::unix::fs::PermissionsExt;

    let id = temp_id();
    let root = std::env::temp_dir();
    let script = root.join(format!("cockpit-herdr-autostart-{id}.sh"));
    let marker = root.join(format!("cockpit-herdr-autostart-{id}.ready"));
    let log = root.join(format!("cockpit-herdr-autostart-{id}.log"));
    let schema = include_str!("fixtures/herdr-0.8.2-protocol-20-schema-1.json");
    let script_body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$*\" in\n  *'status server --json') if [ -f '{}' ]; then printf '%s' '{{\"version\":\"0.8.2\",\"protocol\":20}}'; else printf '%s' '{{\"status\":\"not_running\",\"running\":false,\"version\":null,\"protocol\":null}}'; fi;;\n  *'api schema --json') printf '%s' '{}';;\n  *server) touch '{}';;\nesac\n",
        log.display(),
        marker.display(),
        schema.replace('\'', "'\\''"),
        marker.display(),
    );
    fs::write(&script, script_body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&script, permissions).unwrap();

    let config = HerdrCliConfig::from_options(Some(script.clone()), None, None).unwrap();
    let result = HerdrCliAdapter::new(config)
        .with_server_autostart()
        .inspect()
        .await
        .unwrap();

    assert!(matches!(result, HerdrCompatibility::Compatible { .. }));
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "status server --json",
            "status server --json",
            "server",
            "status server --json",
            "api schema --json"
        ]
    );

    drop(fs::remove_file(marker));
    drop(fs::remove_file(log));
    drop(fs::remove_file(script));
}

#[cfg(unix)]
#[tokio::test]
async fn version_protocol_schema_and_method_mismatches_are_incompatible() {
    let schema = r#"{"schema_version":1,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}},{"properties":{"method":{"const":"session.snapshot"}}},{"properties":{"method":{"const":"events.subscribe"}}}]}}}"#;
    assert!(
        matches!(inspect(r#"{"version":"0.7.0","protocol":20}"#, schema).await, HerdrCompatibility::Incompatible { identity: None, code, .. } if code == "version_mismatch")
    );
    assert!(
        matches!(inspect(r#"{"version":"0.8.2","protocol":22}"#, schema).await, HerdrCompatibility::Incompatible { identity: None, code, .. } if code == "protocol_mismatch")
    );
    assert!(
        matches!(inspect(r#"{"version":"0.8.2","protocol":20}"#, r#"{"schema_version":2,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}},{"properties":{"method":{"const":"session.snapshot"}}},{"properties":{"method":{"const":"events.subscribe"}}}]}}}"#).await, HerdrCompatibility::Incompatible { identity: Some(identity), code, .. } if identity.schema_version == 2 && code == "schema_version_mismatch")
    );
    assert!(
        matches!(inspect(r#"{"version":"0.8.2","protocol":20}"#, r#"{"schema_version":1,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}}]}}}"#).await, HerdrCompatibility::Incompatible { code, .. } if code == "missing_methods")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_output_and_missing_executable_are_unavailable() {
    let malformed_script = fixture_script("not-json", "{}");
    let malformed =
        HerdrCliConfig::from_options(Some(malformed_script.clone()), None, None).unwrap();
    let malformed_result = HerdrCliAdapter::new(malformed).inspect().await.unwrap_err();
    assert_eq!(malformed_result.code, "malformed_json");

    drop(fs::remove_file(malformed_script));

    let missing =
        HerdrCliConfig::from_options(Some(PathBuf::from("/definitely/missing/herdr")), None, None)
            .unwrap();
    let missing_result = HerdrCliAdapter::new(missing).inspect().await.unwrap_err();
    assert_eq!(missing_result.code, "execution_failed");
}
#[cfg(unix)]
#[tokio::test]
async fn nested_decoys_do_not_supply_identity_or_methods() {
    let script = fixture_script(
        r#"{"nested":{"version":"0.8.2","protocol":20}}"#,
        r#"{"schema_version":1,"methods":["ping","session.snapshot","events.subscribe"]}"#,
    );
    let config = HerdrCliConfig::from_options(Some(script.clone()), None, None).unwrap();
    let error = HerdrCliAdapter::new(config).inspect().await.unwrap_err();
    assert_eq!(error.code, "malformed_json");
    drop(fs::remove_file(script));
}

#[cfg(unix)]
#[tokio::test]
async fn records_fixed_argv_and_socket_environment() {
    use std::os::unix::fs::PermissionsExt;
    let id = temp_id();
    let root = std::env::temp_dir();
    let script = root.join(format!("cockpit-herdr-record-{id}.sh"));
    let log = root.join(format!("cockpit-herdr-record-{id}.log"));
    let schema = r#"{"schema_version":1,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}},{"properties":{"method":{"const":"session.snapshot"}}},{"properties":{"method":{"const":"events.subscribe"}}},{"properties":{"method":{"const":"pane.read"}}}]}}}"#;
    let script_body = format!(
        "#!/bin/sh\nprintf '%s|%s\\n' \"$*\" \"${{HERDR_SOCKET_PATH-<inherited>}}\" >> '{}'\ncase \"$*\" in *'status server --json') printf '%s' '{{\"version\":\"0.8.2\",\"protocol\":20}}';; *'api schema --json') printf '%s' '{}';; esac\n",
        log.display(),
        schema.replace('\'', "'\\''")
    );
    fs::write(&script, script_body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&script, permissions).unwrap();

    let session_config = HerdrCliConfig::from_options_with_environment(
        Some(script.clone()),
        Some("named".into()),
        None,
        &BTreeMap::new(),
    )
    .unwrap();
    let _ = HerdrCliAdapter::new(session_config).inspect().await;
    let lines = fs::read_to_string(&log).unwrap();
    let mut recorded = lines.lines();
    assert_eq!(
        recorded.next().unwrap().split_once('|').unwrap().0,
        "--session named status server --json"
    );
    assert_eq!(
        recorded.next().unwrap().split_once('|').unwrap().0,
        "--session named api schema --json"
    );
    drop(fs::remove_file(&log));

    let socket_config = HerdrCliConfig::from_options_with_environment(
        Some(script.clone()),
        Some("named".into()),
        Some(PathBuf::from("/tmp/explicit-herdr.sock")),
        &BTreeMap::new(),
    )
    .unwrap();
    let _ = HerdrCliAdapter::new(socket_config).inspect().await;
    let lines = fs::read_to_string(&log).unwrap();
    let mut recorded = lines.lines();
    assert_eq!(
        recorded.next(),
        Some("status server --json|/tmp/explicit-herdr.sock")
    );
    assert_eq!(
        recorded.next(),
        Some("api schema --json|/tmp/explicit-herdr.sock")
    );
    drop(fs::remove_file(log));
    drop(fs::remove_file(script));
}

#[test]
fn options_override_environment_without_mutating_process_environment() {
    let mut env = BTreeMap::new();
    env.insert("COCKPIT_HERDR_EXECUTABLE".into(), "env-herdr".into());
    env.insert("COCKPIT_HERDR_SESSION".into(), "env-session".into());
    let config = HerdrCliConfig::from_options_with_environment(
        Some(PathBuf::from("option-herdr")),
        Some("option-session".into()),
        None,
        &env,
    )
    .unwrap();
    assert_eq!(config.executable, PathBuf::from("option-herdr"));
    assert_eq!(config.session.as_deref(), Some("option-session"));
}

#[cfg(unix)]
#[test]
fn default_executable_is_found_outside_a_desktop_path() {
    use std::os::unix::fs::PermissionsExt;

    let id = temp_id();
    let home = std::env::temp_dir().join(format!("cockpit-herdr-home-{id}"));
    let executable = home.join(".linuxbrew/bin/herdr");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "#!/bin/sh\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).unwrap();

    let mut env = BTreeMap::new();
    env.insert("HOME".into(), home.display().to_string());
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    let config = HerdrCliConfig::from_options_with_environment(None, None, None, &env).unwrap();

    assert_eq!(config.executable(), executable);

    drop(fs::remove_dir_all(home));
}

#[test]
fn custom_socket_requires_and_accepts_an_explicit_valid_session() {
    let mut env = BTreeMap::new();
    env.insert("COCKPIT_HERDR_SOCKET".into(), "/tmp/herdr.sock".into());
    let config =
        HerdrCliConfig::from_options_with_environment(None, Some("named".into()), None, &env)
            .unwrap();
    assert_eq!(config.session(), Some("named"));
    assert_eq!(
        config.socket(),
        Some(std::path::Path::new("/tmp/herdr.sock"))
    );

    let error = HerdrCliConfig::from_options_with_environment(None, None, None, &env).unwrap_err();
    assert_eq!(error.code, "missing_socket_session");
}
#[test]
fn incompatible_without_full_identity_serializes_null_identity() {
    let response = HerdrCompatibility::Incompatible {
        identity: None,
        code: "protocol_mismatch".into(),
        message: "expected Herdr protocol 20".into(),
    };
    let value = serde_json::to_value(response).unwrap();
    assert!(
        value
            .get("identity")
            .is_some_and(serde_json::Value::is_null)
    );
}
