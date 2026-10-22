use std::{path::PathBuf, process::Output};

use serde_json::Value;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-tags-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn command(&self, arguments: &[&str]) -> std::process::Command {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cockpit"));
        command.args(arguments)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("COCKPIT_HERDR_EXECUTABLE", self.root.join("no-herdr"));
        for name in [
            "HERDR_ENV", "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_SESSION_NAME",
            "COCKPIT_HERDR_SESSION", "COCKPIT_HERDR_SOCKET", "COCKPIT_CONFIG_PATH", "COCKPIT_CONFIG",
        ] {
            command.env_remove(name);
        }
        command
    }

    fn cli(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().unwrap()
    }

    fn assert_error(&self, arguments: &[&str], code: &str) {
        let output = self.cli(arguments);
        assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["error"]["code"], code);
        assert!(response["error"]["message"].as_str().is_some_and(|message| !message.is_empty()));
        assert!(String::from_utf8_lossy(&output.stderr).starts_with(&format!("error: {code}:")));
        // Usage failures must not contact Herdr or initialise a tag store.
        assert!(!self.root.join("state").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn tags_refuses_ambient_default_session_and_incomplete_explicit_targets() {
    let fixture = Fixture::new();
    fixture.assert_error(&["tags", "list"], "tags_target_required");
    fixture.assert_error(&["tags", "--current", "add", "review"], "tags_target_required");
    fixture.assert_error(&["tags", "--space", "w1", "list"], "tags_usage");
    fixture.assert_error(&["tags", "--space", "w1", "--herdr-session", "fixture", "list"], "tags_usage");
    fixture.assert_error(&["tags", "--space", "w1", "--herdr-socket", "/fixture.sock", "list"], "tags_usage");
    fixture.assert_error(&["tags", "--space", "w1", "--herdr-session", "fixture", "--herdr-socket", "", "list"], "tags_usage");
    fixture.assert_error(&["tags", "list", "--all"], "tags_usage");
    // A resolved session alone is not permission to silently select a Space.
    fixture.assert_error(&["tags", "--herdr-session", "fixture", "--herdr-socket", "/fixture.sock", "list"], "tags_target_required");
}

#[test]
fn tags_conflicting_targets_and_missing_payloads_use_json_errors() {
    let fixture = Fixture::new();
    fixture.assert_error(&["tags", "--current", "--space", "w1", "list"], "tags_usage");
    fixture.assert_error(&["tags", "add"], "tags_usage");
    fixture.assert_error(&["tags", "remove"], "tags_usage");
    fixture.assert_error(&["tags"], "tags_usage");
    fixture.assert_error(&["tags", "list", "--all", "--space", "w1", "--herdr-session", "fixture", "--herdr-socket", "/fixture.sock"], "tags_usage");
    fixture.assert_error(&["tags", "list", "--all", "--current"], "tags_usage");
    fixture.assert_error(&["tags", "add", "review", "--all"], "tags_usage");
}

#[test]
fn root_and_tags_help_make_tagging_discoverable_without_initialising_state() {
    let fixture = Fixture::new();
    let root = fixture.cli(&["--help"]);
    assert!(root.status.success());
    assert!(String::from_utf8_lossy(&root.stdout).contains("cockpit-cli tags --help"));
    let tags = fixture.cli(&["tags", "--help"]);
    assert!(tags.status.success());
    let help = String::from_utf8_lossy(&tags.stdout);
    for text in ["list", "add", "remove", "cockpit-cli tags add \"needs review\"", "--space", "--current", "list --all"] {
        assert!(help.contains(text), "missing {text} in help");
    }
    assert!(!fixture.root.join("state").exists());
}

#[tokio::test]
async fn cli_mutations_are_idempotent_and_session_listing_keeps_the_catalog() {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let fixture = Fixture::new();
    let executable = fixture.root.join("herdr");
    std::fs::write(executable.with_extension("schema"), include_str!(
        "../../cockpit-herdr/tests/fixtures/herdr-0.9.0-protocol-22-schema-1.json"
    )).unwrap();
    std::fs::write(&executable, r#"#!/bin/sh
case "$*" in
  *'status server --json') printf '%s\n' '{"version":"0.9.0","protocol":22}';;
  *'api schema --json') cat "$0.schema";;
  *) exit 1;;
esac
"#).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = fixture.root.join("herdr.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    // Reuse the adapter's protocol fixture rather than inventing a second snapshot shape.
    let snapshot: Value = serde_json::from_str(include_str!(
        "../../cockpit-herdr/tests/fixtures/session-snapshot.json"
    )).unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "ping" => serde_json::json!({"type": "pong", "version": "0.9.0", "protocol": 22}),
                "session.snapshot" => snapshot["result"].clone(),
                method => panic!("unexpected Herdr method {method}"),
            };
            let response = serde_json::json!({"id": request["id"], "result": result});
            reader.into_inner().write_all(format!("{response}\n").as_bytes()).await.unwrap();
        }
    });
    async fn call(fixture: &Fixture, executable: &std::path::Path, socket: &std::path::Path, args: &[&str]) -> Value {
        let mut command = fixture.command(&[
            "tags", "--herdr-session", "fixture", "--herdr-socket", socket.to_str().unwrap(),
        ]);
        command.args(args).env("COCKPIT_HERDR_EXECUTABLE", executable);
        let output = tokio::process::Command::from(command).output().await.unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }
    let first = call(&fixture, &executable, &socket, &["--space", "space-a", "add", "needs review"]).await;
    assert_eq!(first["changed"], true);
    assert_eq!(first["tags"], serde_json::json!(["needs review"]));
    assert_eq!(first["session_id"], "fixture");
    assert_eq!(first["space_id"], "space-a");
    assert!(!first["server_instance"].as_str().unwrap().is_empty());
    let duplicate = call(&fixture, &executable, &socket, &["--space", "space-a", "add", "Needs  Review"]).await;
    assert_eq!(duplicate["changed"], false);
    assert_eq!(duplicate["tags"], first["tags"]);
    let listed = call(&fixture, &executable, &socket, &["--space", "space-a", "list"]).await;
    assert_eq!(listed, duplicate);
    let session = call(&fixture, &executable, &socket, &["list", "--all"]).await;
    assert!(session.get("space_id").is_none());
    assert_eq!(session["spaces"], serde_json::json!([{"space_id": "space-a", "tags": ["needs review"]}]));
    let removed = call(&fixture, &executable, &socket, &["--space", "space-a", "remove", "NEEDS REVIEW"]).await;
    assert_eq!(removed["changed"], true);
    assert_eq!(removed["tags"], serde_json::json!([]));
    assert_eq!(removed["catalog"], first["catalog"]);
    let absent = call(&fixture, &executable, &socket, &["--space", "space-a", "remove", "needs review"]).await;
    assert_eq!(absent["changed"], false);
    let empty = call(&fixture, &executable, &socket, &["list", "--all"]).await;
    assert_eq!(empty["spaces"], serde_json::json!([]));
    assert_eq!(empty["catalog"], first["catalog"]);
    server.abort();
}
