use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use cockpit_core::HerdrAdapter;
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_protocol::v1::HerdrCompatibility;
#[cfg(unix)]
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(unix)]
static TRAMPOLINE_USERS: AtomicUsize = AtomicUsize::new(0);
#[cfg(unix)]
static TRAMPOLINE_LOCK: Mutex<()> = Mutex::new(());
#[cfg(unix)]
static TRAMPOLINE: OnceLock<PathBuf> = OnceLock::new();

#[cfg(unix)]
fn temp_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(unix)]
fn shared_trampoline() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let _guard = TRAMPOLINE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let path = TRAMPOLINE
        .get_or_init(|| {
            std::env::temp_dir().join(format!(
                "cockpit-herdr-trampoline-{}.sh",
                std::process::id()
            ))
        })
        .clone();
    if !path.exists() {
        fs::write(
            &path,
            "#!/bin/sh\nfixture=\"${0%.exec}.data\"\n. \"$fixture\"\n",
        )
        .unwrap();
    }
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o555);
    fs::set_permissions(&path, permissions).unwrap();
    TRAMPOLINE_USERS.fetch_add(1, Ordering::Relaxed);
    path
}

#[cfg(unix)]
struct Fixture {
    executable: PathBuf,
    data: PathBuf,
    trampoline: PathBuf,
}

#[cfg(unix)]
impl Fixture {
    fn path(&self) -> &Path {
        &self.executable
    }
}

#[cfg(unix)]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.executable);
        let _ = fs::remove_file(&self.data);
        let _guard = TRAMPOLINE_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if TRAMPOLINE_USERS.fetch_sub(1, Ordering::Relaxed) == 1 {
            let _ = fs::remove_file(&self.trampoline);
        }
    }
}

#[cfg(unix)]
fn shell_fixture(body: &str) -> Fixture {
    let id = temp_id();
    let executable = std::env::temp_dir().join(format!("cockpit-herdr-test-{id}.exec"));
    let data = std::env::temp_dir().join(format!("cockpit-herdr-test-{id}.data"));
    let trampoline = shared_trampoline();
    fs::write(&data, body).unwrap();
    std::os::unix::fs::symlink(&trampoline, &executable).unwrap();
    Fixture {
        executable,
        data,
        trampoline,
    }
}

#[cfg(unix)]
fn fixture_script(status: &str, schema: &str) -> Fixture {
    let script = format!(
        "case \"$*\" in *'status server --json') printf '%s' '{}';; *'api schema --json') printf '%s' '{}';; esac\n",
        status.replace('\'', "'\\''"),
        schema.replace('\'', "'\\''")
    );
    shell_fixture(&script)
}

#[cfg(unix)]
fn process_fixture(body: &str) -> Fixture {
    shell_fixture(body)
}

#[cfg(unix)]
async fn inspect(status: &str, schema: &str) -> HerdrCompatibility {
    let script = fixture_script(status, schema);
    let config =
        HerdrCliConfig::from_options(Some(script.path().to_path_buf()), None, None).unwrap();
    HerdrCliAdapter::new(config)
        .inspect()
        .await
        .unwrap_or_else(|error| panic!("unexpected inspection failure: {}", error))
}

#[cfg(unix)]
fn assert_elapsed(start: Instant, max: Duration) {
    assert!(
        start.elapsed() <= max,
        "operation exceeded {:?}: {:?}",
        max,
        start.elapsed()
    );
}
#[cfg(unix)]
struct ChildFixture {
    fixture: Fixture,
    parent: PathBuf,
    descendant: PathBuf,
    ready: PathBuf,
}

#[cfg(unix)]
impl ChildFixture {
    fn executable(&self) -> PathBuf {
        self.fixture.path().to_path_buf()
    }
}

#[cfg(unix)]
impl Drop for ChildFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.parent);
        let _ = fs::remove_file(&self.descendant);
        let _ = fs::remove_file(&self.ready);
    }
}

#[cfg(unix)]
fn child_fixture(body: &str) -> ChildFixture {
    let id = temp_id();
    let root = std::env::temp_dir();
    let parent = root.join(format!("cockpit-herdr-child-{id}.parent"));
    let descendant = root.join(format!("cockpit-herdr-child-{id}.descendant"));
    let ready = root.join(format!("cockpit-herdr-child-{id}.ready"));
    let body = format!(
        "printf '%s\\n' \"$$\" >> '{}'\nsleep 30 & descendant=$!\nprintf '%s\\n' \"$descendant\" >> '{}'\ntouch '{}'\n{}",
        parent.display(),
        descendant.display(),
        ready.display(),
        body
    );
    let fixture = process_fixture(&body);
    ChildFixture {
        fixture,
        parent,
        descendant,
        ready,
    }
}

#[cfg(unix)]
async fn wait_ready(path: &Path) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if path.is_file() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("child fixture did not become ready");
}

#[cfg(target_os = "linux")]
fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "linux")]
fn process_is_live(pid: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .is_some_and(|state| state != "Z")
}

#[cfg(target_os = "linux")]
async fn wait_processes_gone(pids: &[(u32, u32)]) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if pids.iter().all(|(parent, descendant)| {
                !process_exists(*parent) && !process_is_live(*descendant)
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("child process cleanup exceeded 2 seconds");
}

#[cfg(not(target_os = "linux"))]
async fn wait_processes_gone(_pids: &[(u32, u32)]) {
    eprintln!("process cleanup assertions unsupported on this platform");
}

#[cfg(unix)]
#[tokio::test]
async fn installed_schema_fixture_is_compatible() {
    let schema = include_str!("fixtures/herdr-0.9.0-protocol-22-schema-1.json");
    let status = r#"{"version":"0.9.1","protocol":22}"#;
    assert!(
        matches!(inspect(status, schema).await, HerdrCompatibility::Compatible { identity } if identity.version == "0.9.1" && identity.protocol == 22 && identity.schema_version == 1)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn autostart_launches_a_missing_server_before_inspection() {
    let id = temp_id();
    let root = std::env::temp_dir();
    let marker = root.join(format!("cockpit-herdr-autostart-{id}.ready"));
    let log = root.join(format!("cockpit-herdr-autostart-{id}.log"));
    let schema = include_str!("fixtures/herdr-0.9.0-protocol-22-schema-1.json");
    let script_body = format!(
        "printf '%s\\n' \"$*\" >> '{}'\ncase \"$*\" in\n  *'status server --json') if [ -f '{}' ]; then printf '%s' '{{\"version\":\"0.9.0\",\"protocol\":22}}'; else printf '%s' '{{\"status\":\"not_running\",\"running\":false,\"version\":null,\"protocol\":null}}'; fi;;\n  *'api schema --json') printf '%s' '{}';;\n  *server) touch '{}';;\nesac\n",
        log.display(),
        marker.display(),
        schema.replace('\'', "'\\''"),
        marker.display(),
    );
    let script = process_fixture(&script_body);
    let config =
        HerdrCliConfig::from_options(Some(script.path().to_path_buf()), None, None).unwrap();
    let started = Instant::now();
    let result = HerdrCliAdapter::new(config)
        .with_server_autostart()
        .inspect()
        .await
        .unwrap();
    assert_elapsed(started, Duration::from_secs(5));

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
}
#[cfg(unix)]
#[tokio::test]
async fn bounded_autostart_case_timeout_reaps_children() {
    let script = child_fixture(
        "case \"$*\" in\n  *'status server --json') printf '%s' '{\"status\":\"not_running\",\"running\":false,\"version\":null,\"protocol\":null}'; kill \"$descendant\"; wait \"$descendant\";;\n  *'api schema --json') printf '%s' '{}'; kill \"$descendant\"; wait \"$descendant\";;\n  *server) while :; do sleep 1; done;;\nesac",
    );
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let started = Instant::now();
    let adapter = HerdrCliAdapter::new(config).with_server_autostart();
    let task = tokio::spawn(async move { adapter.inspect().await });
    wait_ready(&script.ready).await;
    wait_pid_count(&script.parent, &script.descendant, 2).await;
    let error = tokio::time::timeout(Duration::from_secs(7), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(started.elapsed() >= Duration::from_secs(4));
    assert_elapsed(started, Duration::from_secs(7));
    assert_eq!(error.code, "server_start_timeout");
    let pids = child_pids(&script);
    wait_processes_gone(&pids).await;
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_autostart_case_caller_cancellation_reaps_children() {
    let script = child_fixture(
        "case \"$*\" in\n  *'status server --json') printf '%s' '{\"status\":\"not_running\",\"running\":false,\"version\":null,\"protocol\":null}'; kill \"$descendant\"; wait \"$descendant\";;\n  *'api schema --json') printf '%s' '{}'; kill \"$descendant\"; wait \"$descendant\";;\n  *server) while :; do sleep 1; done;;\nesac",
    );
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let adapter = HerdrCliAdapter::new(config).with_server_autostart();
    let started = Instant::now();
    let task = tokio::spawn(async move { adapter.inspect().await });
    wait_ready(&script.ready).await;
    wait_pid_count(&script.parent, &script.descendant, 2).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_elapsed(started, Duration::from_secs(2));
    let pids = child_pids(&script);
    wait_processes_gone(&pids).await;
}

#[cfg(unix)]
#[tokio::test]
async fn protocol_schema_and_method_mismatches_are_incompatible() {
    let schema = r#"{"schema_version":1,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}},{"properties":{"method":{"const":"session.snapshot"}}},{"properties":{"method":{"const":"events.subscribe"}}}]}}}"#;
    assert!(
        matches!(inspect(r#"{"version":"0.9.0","protocol":20}"#, schema).await, HerdrCompatibility::Incompatible { identity: None, code, .. } if code == "protocol_mismatch")
    );
    assert!(
        matches!(inspect(r#"{"version":"0.9.0","protocol":22}"#, r#"{"schema_version":2,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}},{"properties":{"method":{"const":"session.snapshot"}}},{"properties":{"method":{"const":"events.subscribe"}}}]}}}"#).await, HerdrCompatibility::Incompatible { identity: Some(identity), code, .. } if identity.schema_version == 2 && code == "schema_version_mismatch")
    );
    assert!(
        matches!(inspect(r#"{"version":"0.9.0","protocol":22}"#, r#"{"schema_version":1,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}}]}}}"#).await, HerdrCompatibility::Incompatible { code, .. } if code == "missing_methods")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_output_and_missing_executable_are_unavailable() {
    let malformed_script = fixture_script("not-json", "{}");
    let malformed =
        HerdrCliConfig::from_options(Some(malformed_script.path().to_path_buf()), None, None)
            .unwrap();
    let started = Instant::now();
    let malformed_result = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(malformed).inspect(),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(malformed_result.code, "malformed_json");

    let missing =
        HerdrCliConfig::from_options(Some(PathBuf::from("/definitely/missing/herdr")), None, None)
            .unwrap();
    let started = Instant::now();
    let missing_result = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(missing).inspect(),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(missing_result.code, "execution_failed");
}
#[cfg(unix)]
#[tokio::test]
async fn nested_decoys_do_not_supply_identity_or_methods() {
    let script = fixture_script(
        r#"{"nested":{"version":"0.8.2","protocol":20}}"#,
        r#"{"schema_version":1,"methods":["ping","session.snapshot","events.subscribe"]}"#,
    );
    let config =
        HerdrCliConfig::from_options(Some(script.path().to_path_buf()), None, None).unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(config).inspect(),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(error.code, "malformed_json");
}
#[cfg(unix)]
#[tokio::test]
async fn records_fixed_argv_and_socket_environment() {
    let id = temp_id();
    let root = std::env::temp_dir();
    let log = root.join(format!("cockpit-herdr-record-{id}.log"));
    let schema = r#"{"schema_version":1,"schemas":{"request":{"oneOf":[{"properties":{"method":{"const":"ping"}}},{"properties":{"method":{"const":"session.snapshot"}}},{"properties":{"method":{"const":"events.subscribe"}}},{"properties":{"method":{"const":"pane.read"}}}]}}}"#;
    let script_body = format!(
        "printf '%s|%s\\n' \"$*\" \"${{HERDR_SOCKET_PATH-<inherited>}}\" >> '{}'\ncase \"$*\" in *'status server --json') printf '%s' '{{\"version\":\"0.9.0\",\"protocol\":22}}';; *'api schema --json') printf '%s' '{}';; esac\n",
        log.display(),
        schema.replace('\'', "'\\''")
    );
    let script = process_fixture(&script_body);

    let session_config = HerdrCliConfig::from_options_with_environment(
        Some(script.path().to_path_buf()),
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
        Some(script.path().to_path_buf()),
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
    let id = temp_id();
    let home = std::env::temp_dir().join(format!("cockpit-herdr-home-{id}"));
    let executable = home.join(".linuxbrew/bin/herdr");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    let fixture = process_fixture("");
    std::os::unix::fs::symlink(fixture.path(), &executable).unwrap();

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
        message: "expected Herdr protocol 22".into(),
    };
    let value = serde_json::to_value(response).unwrap();
    assert!(
        value
            .get("identity")
            .is_some_and(serde_json::Value::is_null)
    );
}

#[cfg(unix)]
fn child_pids(fixture: &ChildFixture) -> Vec<(u32, u32)> {
    let parents = fs::read_to_string(&fixture.parent).unwrap();
    let descendants = fs::read_to_string(&fixture.descendant).unwrap();
    parents
        .lines()
        .zip(descendants.lines())
        .map(|(parent, descendant)| (parent.parse().unwrap(), descendant.parse().unwrap()))
        .collect()
}

#[cfg(unix)]
async fn wait_pid_count(parent: &Path, descendant: &Path, count: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let parents = fs::read_to_string(parent)
                .map(|contents| contents.lines().count() >= count)
                .unwrap_or(false);
            let descendants = fs::read_to_string(descendant)
                .map(|contents| contents.lines().count() >= count)
                .unwrap_or(false);
            if parents && descendants {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("child fixture did not reach the expected process count");
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_child_case_stdout_overflow_kills_held_stderr() {
    let script = child_fixture("printf '%1048577s\\n' ''");
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let pids = {
        let started = Instant::now();
        let adapter = HerdrCliAdapter::new(config);
        let task = tokio::spawn(async move { adapter.inspect().await });
        wait_ready(&script.ready).await;
        let pids = child_pids(&script);
        let error = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_elapsed(started, Duration::from_secs(2));
        assert_eq!(error.code, "bounded_output");
        pids
    };
    wait_processes_gone(&pids).await;
}
#[cfg(unix)]
#[tokio::test]
async fn bounded_child_case_stderr_overflow_kills_held_stdout() {
    let script = child_fixture("printf '%1048577s\\n' '' >&2");
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let started = Instant::now();
    let adapter = HerdrCliAdapter::new(config);
    let task = tokio::spawn(async move { adapter.inspect().await });
    wait_ready(&script.ready).await;
    let pids = child_pids(&script);
    let error = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(error.code, "bounded_output");
    wait_processes_gone(&pids).await;
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_child_case_never_exits_hits_total_deadline() {
    let script = child_fixture("while :; do sleep 1; done");
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let started = Instant::now();
    let adapter = HerdrCliAdapter::new(config);
    let task = tokio::spawn(async move { adapter.inspect().await });
    wait_ready(&script.ready).await;
    let pids = child_pids(&script);
    let error = tokio::time::timeout(Duration::from_secs(7), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(started.elapsed() >= Duration::from_secs(4));
    assert_elapsed(started, Duration::from_secs(7));
    assert_eq!(error.code, "execution_timeout");
    wait_processes_gone(&pids).await;
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_child_case_malformed_json_after_clean_exit() {
    let script =
        child_fixture("kill \"$descendant\"; wait \"$descendant\"; printf '%s' 'not-json'");
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(config).inspect(),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(error.code, "malformed_json");
    let pids = child_pids(&script);
    wait_processes_gone(&pids).await;
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_child_case_nonzero_exit() {
    let script = child_fixture("kill \"$descendant\"; wait \"$descendant\"; exit 7");
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(config).inspect(),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(error.code, "execution_failed");
    let pids = child_pids(&script);
    wait_processes_gone(&pids).await;
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_child_case_caller_cancellation() {
    let script = child_fixture("while :; do sleep 1; done");
    let config = HerdrCliConfig::from_options(Some(script.executable()), None, None).unwrap();
    let adapter = HerdrCliAdapter::new(config);
    let started = Instant::now();
    let task = tokio::spawn(async move { adapter.inspect().await });
    wait_ready(&script.ready).await;
    let pids = child_pids(&script);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_elapsed(started, Duration::from_secs(2));
    wait_processes_gone(&pids).await;
}
