use super::*;
use async_trait::async_trait;
use herdr::QuickAskBinding;
use std::sync::Mutex;

fn request(prompt: &str) -> QuickAskRequest {
    QuickAskRequest {
        server_instance: "server".into(),
        space_id: "space".into(),
        cwd: "/".into(),
        harness: QuickAskHarness::Omp,
        prompt: prompt.into(),
    }
}

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<Vec<String>>>,
    bind_error: bool,
    create_error: bool,
}

#[async_trait]
impl QuickAskHerdr for Fake {
    async fn bind(&self, session_id: &str, server_instance: &str, space_id: &str) -> Result<QuickAskBinding, InspectionError> {
        self.calls.lock().unwrap().push(vec!["bind".into(), session_id.into(), server_instance.into(), space_id.into()]);
        if self.bind_error { return Err(InspectionError::new("bind_failed", "binding failed")); }
        Ok(QuickAskBinding { session_id: session_id.into(), endpoint_identity: "endpoint".into(), server_instance: server_instance.into(), space_id: space_id.into() })
    }

    async fn create_tab(&self, binding: &QuickAskBinding, cwd: &str, label: &str, kind: &str, prompt: &str, _path: &str) -> Result<QuickAskLocation, InspectionError> {
        self.calls.lock().unwrap().push(vec!["create".into(), binding.session_id.clone(), binding.server_instance.clone(), binding.space_id.clone(), cwd.into(), label.into(), kind.into(), prompt.into()]);
        if self.create_error { return Err(InspectionError::new("create_failed", "creation failed")); }
        Ok(QuickAskLocation { space_id: binding.space_id.clone(), tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: "terminal".into(), tab_label: label.into() })
    }
}

#[tokio::test]
async fn ask_binds_and_creates_once_with_literal_prompt_and_returns_location() {
    let adapter = Arc::new(Fake::default());
    let service = QuickAskService::new(adapter.clone());
    let prompt = "\n  What   happens?  \nsecond line";
    let location = service.ask("session", request(prompt)).await.unwrap();
    assert_eq!(location.space_id, "space");
    assert_eq!(location.tab_id, "tab");
    assert_eq!(location.pane_id, "pane");
    assert_eq!(location.terminal_id, "terminal");
    assert_eq!(location.tab_label, "What happens?");
    assert_eq!(*adapter.calls.lock().unwrap(), vec![
        vec!["bind", "session", "server", "space"],
        vec!["create", "session", "server", "space", "/", "What happens?", "omp", prompt],
    ]);
}

#[tokio::test]
async fn each_harness_and_each_request_create_a_tab() {
    let adapter = Arc::new(Fake::default());
    let service = QuickAskService::new(adapter.clone());
    for (harness, kind, _) in HARNESS {
        let mut req = request("question");
        req.harness = harness;
        service.ask("session", req).await.unwrap();
        let calls = adapter.calls.lock().unwrap();
        assert_eq!(calls.last().unwrap()[6], kind);
    }
    assert_eq!(adapter.calls.lock().unwrap().len(), 12);
}

#[test]
fn labels_use_first_nonempty_line_and_truncate_unicode_characters() {
    assert_eq!(tab_label("\n  hello  world  \nignored"), "hello world");
    assert_eq!(tab_label(&"é".repeat(32)), "é".repeat(32));
    assert_eq!(tab_label(&"é".repeat(33)), format!("{}…", "é".repeat(32)));
}

#[tokio::test]
async fn rejects_invalid_requests_before_binding() {
    let adapter = Arc::new(Fake::default());
    let service = QuickAskService::new(adapter.clone());
    assert!(service.ask("../invalid", request("question")).await.is_err());
    let mut req = request("question");
    req.space_id = "../invalid".into();
    assert!(service.ask("session", req).await.is_err());
    for cwd in ["relative", "/this-directory-should-not-exist-cockpit-quick-ask"] {
        let mut req = request("question");
        req.cwd = cwd.into();
        assert_eq!(service.ask("session", req).await.unwrap_err().code, "quick_ask_invalid_cwd");
    }
    for prompt in ["", " \n ", "tab\there", "return\rhere", "null\0here", "\u{7f}"] {
        assert_eq!(service.ask("session", request(prompt)).await.unwrap_err().code, "quick_ask_invalid_prompt");
    }
    assert_eq!(service.ask("session", request(&"x".repeat(16385))).await.unwrap_err().code, "quick_ask_invalid_prompt");
    assert_eq!(service.ask("session", request(&"é".repeat(8193))).await.unwrap_err().code, "quick_ask_invalid_prompt");
    assert!(adapter.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn accepts_prompt_at_byte_limit_and_line_feeds() {
    let adapter = Arc::new(Fake::default());
    let service = QuickAskService::new(adapter);
    service.ask("session", request(&"é".repeat(8192))).await.unwrap();
    service.ask("session", request("first\nsecond")).await.unwrap();
}

#[tokio::test]
async fn adapter_failures_propagate_without_retry() {
    let adapter = Arc::new(Fake { bind_error: true, ..Fake::default() });
    let service = QuickAskService::new(adapter.clone());
    assert_eq!(service.ask("session", request("question")).await.unwrap_err().code, "bind_failed");
    assert_eq!(adapter.calls.lock().unwrap().len(), 1);

    let adapter = Arc::new(Fake { create_error: true, ..Fake::default() });
    let service = QuickAskService::new(adapter.clone());
    assert_eq!(service.ask("session", request("question")).await.unwrap_err().code, "create_failed");
    assert_eq!(adapter.calls.lock().unwrap().len(), 2);
}

#[cfg(unix)]
mod discovery {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::{Path, PathBuf}};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("quick-ask-discovery-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn executable(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[tokio::test]
    async fn shell_probe_frames_path_through_startup_noise() {
        let fixture = Fixture::new();
        let shell = fixture.executable("shell", "#!/bin/sh\nprintf 'noise\\000wrong\\000\\n'\nPATH=/initialized/bin:/usr/bin\nexport PATH\n/bin/sh -c \"$2\"\nprintf 'trailing noise\\n'\n");
        assert_eq!(shell_path(&shell, "/fallback".into()).await, OsString::from("/initialized/bin:/usr/bin"));
    }

    #[tokio::test]
    async fn shell_probe_rejects_failed_unframed_and_oversized_output() {
        let fixture = Fixture::new();
        let fallback = OsString::from("/fallback");
        let failed = fixture.executable("failed", "#!/bin/sh\n/bin/sh -c \"$2\"\nexit 1\n");
        let unframed = fixture.executable("unframed", "#!/bin/sh\nprintf 'not a path\\000'\n");
        let oversized = fixture.executable("oversized", "#!/bin/sh\n/usr/bin/head -c 65537 /dev/zero\n/bin/sh -c \"$2\"\n");
        for shell in [failed.as_path(), unframed.as_path(), oversized.as_path(), Path::new("/no-such-quick-ask-shell")] {
            assert_eq!(shell_path(shell, fallback.clone()).await, fallback);
        }
    }

    #[tokio::test]
    async fn shell_probe_times_out() {
        let fixture = Fixture::new();
        let shell = fixture.executable("slow", "#!/bin/sh\nexec /bin/sleep 10\n");
        let fallback = OsString::from("/fallback");
        assert_eq!(shell_path(&shell, fallback.clone()).await, fallback);
    }

    #[test]
    fn inactive_nvm_versions_fill_only_missing_clients_in_numeric_order() {
        let fixture = Fixture::new();
        for name in [
            "active/omp", "versions/node/v24.9.0/bin/node", "versions/node/v24.9.0/bin/pi",
            "versions/node/v24.15.0/bin/node", "versions/node/v24.15.0/bin/omp",
            "versions/node/v24.15.0/bin/pi", "versions/node/v24.15.0/bin/claude",
            "versions/node/v23.1.0/bin/node", "versions/node/v23.1.0/bin/copilot",
            "versions/node/v25.0.0/bin/copilot", "versions/node/v099.0.0/bin/node",
            "versions/node/v099.0.0/bin/copilot",
        ] { fixture.executable(name, "#!/bin/sh\nexit 0\n"); }
        let active = fixture.0.join("active");
        let newer = fixture.0.join("versions/node/v24.15.0/bin");
        let older = fixture.0.join("versions/node/v23.1.0/bin");
        let path = include_nvm_clients(active.as_os_str().to_os_string(), &fixture.0);
        assert_eq!(std::env::split_paths(&path).collect::<Vec<_>>(), vec![active, newer, older]);
        for client in ["omp", "pi", "claude", "copilot"] { assert!(executable_found(client, &path)); }
        assert!(!executable_found("codex", &path));
    }
}
