use super::*;
use crate::credentials::{CredentialVault, MemoryVault};
use cockpit_protocol::browser::{BrowserCleanupRetryRequest, BrowserLegacyCandidateState, BrowserLegacyKeepRequest, BrowserLegacyRemovalRequest};
use serde_json::json;
use std::os::unix::fs::{PermissionsExt, symlink};
use tokio::net::UnixListener;

struct Adapter { source: parking_lot::Mutex<BrowserHerdrSnapshot>, failed: AtomicBool }
#[async_trait]
impl BrowserHerdrAdapter for Adapter {
    async fn browser_snapshot(&self, _: &str) -> Result<BrowserHerdrSnapshot, InspectionError> {
        if self.failed.load(Ordering::Relaxed) { return Err(InspectionError::new("offline", "fixture offline")); }
        Ok(self.source.lock().clone())
    }
}
struct Fixture { root: PathBuf, service: BrowserService, adapter: Arc<Adapter> }
impl Fixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!("cb-life-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let snapshot = serde_json::from_value(json!({
            "session_id":"daily", "server_instance":"fixture", "version":"fixture", "protocol":22,
            "focused_space_id":"w1", "focused_tab_id":"w1:t1", "focused_pane_id":"w1:p1",
            "spaces":[{"id":"w1","label":"Space","number":1,"tab_count":2,"pane_count":2,"focused":true,"agent_status":"idle","git":null}],
            "tabs":[{"id":"w1:t1","space_id":"w1","label":"One","number":1,"pane_count":1,"focused":true,"focused_pane_id":"w1:p1"}, {"id":"w1:t2","space_id":"w1","label":"Two","number":2,"pane_count":1,"focused":false,"focused_pane_id":"w1:p2"}],
            "panes":[{"id":"w1:p1","terminal_id":"term1","space_id":"w1","tab_id":"w1:t1","focused":true,"agent_status":"idle","revision":0}, {"id":"w1:p2","terminal_id":"term2","space_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":0}], "agents":[]
        })).unwrap();
        let adapter = Arc::new(Adapter { source: parking_lot::Mutex::new(BrowserHerdrSnapshot { endpoint_identity: format!("fixture-{}", Uuid::new_v4()), endpoint_path: "/fixture/herdr.sock".into(), snapshot }), failed: AtomicBool::new(false) });
        let cli = root.join("playwright-cli");
        fs::write(&cli, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 0.1.5; else echo 'fixture has no browser'; exit 1; fi\n").unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let service = BrowserService::new(BrowserConfiguration { playwright_cli: cli, default_url: "https://default.test/start".into(), chromium_executable: None, node_executable: None, browser_helper: None, playwright_core: None, feedback_retention_seconds: 3600, feedback_max_store_bytes: 1024 * 1024 }, root.clone(), adapter.clone()).unwrap();
        Self { root, service, adapter }
    }
    fn target(&self, tab: &str) -> BrowserTarget { BrowserTarget { session_id: "daily".into(), tab_id: Some(tab.into()), pane_id: None, endpoint_path: Some("/fixture/herdr.sock".into()) } }
    async fn receipt(&self, tab: &str) -> BrowserReceipt {
        let target = self.service.resolve_target(&self.target(tab)).await.unwrap();
        self.service.load_or_create(&target).unwrap()
    }
    async fn close(&self, tab: &str) -> BrowserResponse { self.service.execute(BrowserRequest { target: self.target(tab), action: BrowserAction::Close }).await.unwrap() }
    fn protected(&self) -> Vec<(PathBuf, Vec<u8>)> {
        ["Library/page.md", "comments/batch.json", "browser/feedback/preserved.txt", "browser/artifacts/preserved.png", "browser/drafts/preserved.txt", "browser/legacy-archive/preserved.txt"].iter().map(|name| {
            let path = self.root.join(name); fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = format!("protected {name}").into_bytes(); fs::write(&path, &bytes).unwrap(); (path, bytes)
        }).collect()
    }
    fn legacy(&self, receipt: &BrowserReceipt) {
        let mut value = serde_json::to_value(receipt).unwrap();
        for name in ["tab_id", "tab_label", "artifacts", "cleanup_reason", "unproven_paths"] { value.as_object_mut().unwrap().remove(name); }
        fs::create_dir_all(self.service.root.join("associations")).unwrap();
        atomic_write_json(&self.service.root.join("associations").join(format!("{}.json", receipt.association_key)), &value).unwrap();
        fs::remove_file(self.service.tab_association_path(&receipt.association_key)).unwrap();
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }
fn assert_protected(items: &[(PathBuf, Vec<u8>)]) { for (path, bytes) in items { assert_eq!(&fs::read(path).unwrap(), bytes, "{}", path.display()); } }

#[tokio::test]
async fn close_disposes_only_its_tab_and_preserves_saved_stores_and_vault() {
    let f = Fixture::new();
    let a = f.receipt("w1:t1").await;
    let b = f.receipt("w1:t2").await;
    assert_ne!(a.association_key, b.association_key);
    assert_ne!(a.profile_path, b.profile_path);
    fs::write(Path::new(&a.profile_path).join("cookies"), b"a cookies").unwrap();
    fs::write(Path::new(&b.profile_path).join("cookies"), b"b cookies").unwrap();
    let b_receipt = fs::read(f.service.tab_association_path(&b.association_key)).unwrap();
    let protected = f.protected();
    let vault = MemoryVault::new(); vault.set("fixture-token", "test", "secret").unwrap();
    let response = f.close("w1:t1").await;
    assert_eq!(response.connection, BrowserConnectionState::Closed);
    assert_eq!(response.cleanup, BrowserCleanupState::Done);
    for path in [&a.profile_path, &a.working_directory, &a.config_path] { assert!(!Path::new(path).exists()); }
    assert!(!f.service.tab_association_path(&a.association_key).exists());
    assert_eq!(fs::read(f.service.tab_association_path(&b.association_key)).unwrap(), b_receipt);
    assert_eq!(fs::read(Path::new(&b.profile_path).join("cookies")).unwrap(), b"b cookies");
    assert_eq!(vault.get("fixture-token").unwrap().as_deref(), Some("secret"));
    assert_protected(&protected);
}

#[tokio::test]
async fn cleanup_unlinks_nested_symlinks_without_touching_their_targets() {
    let f = Fixture::new(); let a = f.receipt("w1:t1").await;
    let outside = f.root.join("outside"); fs::create_dir(&outside).unwrap(); fs::write(outside.join("keep"), b"outside").unwrap();
    symlink(&outside, Path::new(&a.profile_path).join("linked-dir")).unwrap();
    symlink(outside.join("keep"), Path::new(&a.profile_path).join("linked-file")).unwrap();
    assert_eq!(f.close("w1:t1").await.cleanup, BrowserCleanupState::Done);
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside");
}

#[tokio::test]
async fn replaced_profile_is_unproven_and_retry_succeeds_only_after_original_returns() {
    let f = Fixture::new(); let a = f.receipt("w1:t1").await;
    let held = f.root.join("held-original"); fs::rename(&a.profile_path, &held).unwrap();
    fs::create_dir(&a.profile_path).unwrap(); fs::write(Path::new(&a.profile_path).join("unrelated"), b"keep").unwrap();
    assert_eq!(f.close("w1:t1").await.cleanup, BrowserCleanupState::Failed);
    let status = f.service.cleanup_status().await.unwrap();
    assert!(status.failures.iter().any(|failure| failure.unproven_paths.contains(&a.profile_path)));
    assert_eq!(fs::read(Path::new(&a.profile_path).join("unrelated")).unwrap(), b"keep");
    fs::rename(&a.profile_path, f.root.join("replacement-kept")).unwrap();
    symlink(f.root.join("replacement-kept"), &a.profile_path).unwrap();
    assert_eq!(f.close("w1:t1").await.cleanup, BrowserCleanupState::Failed);
    assert!(fs::symlink_metadata(&a.profile_path).unwrap().file_type().is_symlink());
    fs::remove_file(&a.profile_path).unwrap(); fs::rename(&held, &a.profile_path).unwrap();
    assert!(f.service.retry_cleanup(BrowserCleanupRetryRequest { association_key: a.association_key }).await.unwrap().failures.is_empty());
    assert_eq!(fs::read(f.root.join("replacement-kept/unrelated")).unwrap(), b"keep");
}

#[tokio::test]
async fn mismatched_receipt_stem_never_deletes_artifacts_and_is_reported() {
    let f = Fixture::new(); let a = f.receipt("w1:t1").await;
    let wrong = "111111111111111111111111";
    fs::rename(f.service.tab_association_path(&a.association_key), f.service.tab_association_path(wrong)).unwrap();
    let status = f.service.cleanup_status().await.unwrap();
    assert!(status.failures.iter().any(|failure| failure.association_key == wrong && failure.unproven_paths.contains(&a.profile_path)));
    f.service.reconcile().await.unwrap();
    assert!(Path::new(&a.profile_path).exists());
}

#[tokio::test]
async fn fresh_absence_allows_close_but_endpoint_change_rejects_it_and_reconcile_retires() {
    let f = Fixture::new(); let a = f.receipt("w1:t1").await;
    {
        let mut source = f.adapter.source.lock();
        source.snapshot.tabs.retain(|tab| tab.id != "w1:t1"); source.snapshot.panes.retain(|pane| pane.tab_id != "w1:t1");
        source.endpoint_identity = "replacement-endpoint".into();
    }
    assert_eq!(f.service.execute(BrowserRequest { target: f.target("w1:t1"), action: BrowserAction::Close }).await.unwrap_err().code, "stale_endpoint");
    assert!(Path::new(&a.profile_path).exists());
    f.adapter.source.lock().endpoint_identity = a.endpoint_identity.clone();
    assert_eq!(f.close("w1:t1").await.cleanup, BrowserCleanupState::Done);
    let b = f.receipt("w1:t2").await;
    f.adapter.failed.store(true, Ordering::Relaxed); f.service.reconcile().await.unwrap(); assert!(Path::new(&b.profile_path).exists());
    f.adapter.failed.store(false, Ordering::Relaxed);
    f.adapter.source.lock().snapshot.panes.clear();
    assert_eq!(f.service.execute(BrowserRequest { target: f.target("w1:t2"), action: BrowserAction::OpenFresh { url: None } }).await.unwrap_err().code, "tab_not_visible");
    f.service.reconcile().await.unwrap(); assert!(!Path::new(&b.profile_path).exists());
}

#[tokio::test]
async fn unreceipted_existing_profile_cannot_become_owned_by_open() {
    let f = Fixture::new();
    let target = f.service.resolve_target(&f.target("w1:t1")).await.unwrap();
    let key = association_key(&target.endpoint_identity, &target.session_id, &target.tab_id);
    let profile = f.service.root.join("profiles").join(&key); fs::create_dir(&profile).unwrap(); fs::write(profile.join("keep"), b"user data").unwrap();
    assert_eq!(f.service.execute(BrowserRequest { target: f.target("w1:t1"), action: BrowserAction::OpenFresh { url: None } }).await.unwrap_err().code, "browser_artifact_unproven");
    assert_eq!(fs::read(profile.join("keep")).unwrap(), b"user data");
    assert!(!f.service.root.join("workspaces").join(key).exists());
}

#[tokio::test]
async fn legacy_cutover_archives_without_deletion_and_operator_revalidates_reviewed_objects() {
    let f = Fixture::new(); let a = f.receipt("w1:t1").await; let b = f.receipt("w1:t2").await;
    fs::write(Path::new(&a.profile_path).join("legacy-data"), b"original").unwrap();
    let protected = f.protected(); f.legacy(&a);
    f.service.retire_legacy_space_associations().await.unwrap();
    let list = f.service.legacy_list().await.unwrap(); let archive = &list.archives[0];
    assert!(archive.session_stopped); assert_eq!(archive.candidates.len(), 3);
    assert!(!f.service.root.join("associations").join(format!("{}.json", a.association_key)).exists());
    assert_eq!(fs::read(Path::new(&a.profile_path).join("legacy-data")).unwrap(), b"original");
    let archive_path = f.service.root.join("legacy-archive").join(format!("{}.json", a.association_key)); let provenance = fs::read(&archive_path).unwrap();
    let restarted = BrowserService::new(
        f.service.configuration.clone(), f.root.clone(), f.adapter.clone(),
    ).unwrap();
    f.adapter.failed.store(true, Ordering::Relaxed);
    assert_eq!(restarted.legacy_list().await.unwrap(), list);
    assert_eq!(
        restarted.resolve_work_scope(&BrowserWorkScope::LegacyArchive {
            association_key: a.association_key.clone(),
        }).await.unwrap().association_key,
        a.association_key,
    );
    f.adapter.failed.store(false, Ordering::Relaxed);
    let reviewed = archive.candidates.iter().find(|c| c.path == a.profile_path).unwrap().clone();
    fs::rename(&a.profile_path, f.root.join("legacy-held")).unwrap(); fs::create_dir(&a.profile_path).unwrap(); fs::write(Path::new(&a.profile_path).join("replacement"), b"not owned").unwrap();
    let changed = f.service.legacy_remove(BrowserLegacyRemovalRequest { association_key: a.association_key.clone(), candidates: vec![reviewed] }).await.unwrap();
    assert_eq!(changed.archives[0].candidates.iter().find(|c| c.path == a.profile_path).unwrap().state, BrowserLegacyCandidateState::Changed);
    assert_eq!(fs::read(Path::new(&a.profile_path).join("replacement")).unwrap(), b"not owned");
    assert_eq!(fs::read(&archive_path).unwrap(), provenance);
    let config = changed.archives[0].candidates.iter().find(|c| c.path == a.config_path).unwrap().clone();
    f.service.legacy_remove(BrowserLegacyRemovalRequest { association_key: a.association_key.clone(), candidates: vec![config] }).await.unwrap(); assert!(!Path::new(&a.config_path).exists());
    f.service.legacy_keep(BrowserLegacyKeepRequest { association_key: a.association_key.clone() }).await.unwrap();
    assert!(Path::new(&a.profile_path).exists()); assert!(Path::new(&a.working_directory).exists());
    assert!(Path::new(&b.profile_path).exists()); assert_protected(&protected);
    assert!(f.service.resolve_work_scope(&BrowserWorkScope::LegacyArchive { association_key: a.association_key }).await.is_err(), "fully resolved empty archive is retired");
}

#[tokio::test]
async fn shutdown_closes_only_this_runtime_receipts() {
    let f = Fixture::new(); let a = f.receipt("w1:t1").await; let mut b = f.receipt("w1:t2").await;
    b.owner_id = "prior-runtime".into(); f.service.store(&b).unwrap();
    f.service.shutdown().await.unwrap();
    assert!(!Path::new(&a.profile_path).exists()); assert!(Path::new(&b.profile_path).exists());
}

/// Live transport fixtures use an exclusively created, uniquely keyed daemon
/// directory. No environment mutation or existing Playwright session is used.
struct LiveFixture { daemon_dir: PathBuf, _socket: UnixListener, cdp: tokio::task::JoinHandle<()> }
impl Drop for LiveFixture { fn drop(&mut self) { self.cdp.abort(); let _ = fs::remove_dir_all(&self.daemon_dir); } }
async fn seed_live(f: &Fixture, receipt: &mut BrowserReceipt) -> LiveFixture {
    let socket_path = f.root.join(format!("daemon-{}.sock", receipt.association_key)); let socket = UnixListener::bind(&socket_path).unwrap();
    let hash = format!("{:x}", Sha1::digest(receipt.working_directory.as_bytes()));
    let daemon_root = if let Some(path) = env::var_os("PLAYWRIGHT_DAEMON_SESSION_DIR").filter(|s| !s.is_empty()) { PathBuf::from(path) } else {
        #[cfg(target_os = "macos")]
        let cache = PathBuf::from(env::var_os("HOME").unwrap()).join("Library/Caches");
        #[cfg(not(target_os = "macos"))]
        let cache = env::var_os("XDG_CACHE_HOME").filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap()).join(".cache"));
        cache.join("ms-playwright/daemon")
    };
    fs::create_dir_all(&daemon_root).unwrap(); let daemon_dir = daemon_root.join(&hash[..16]); fs::create_dir(&daemon_dir).unwrap();
    let daemon_path = daemon_dir.join(format!("{}.session", receipt.playwright_session));
    let daemon = json!({"workspaceDir":receipt.working_directory,"socketPath":socket_path,"browser":{"userDataDir":receipt.profile_path},"fixture_generation":"before"});
    let bytes = serde_json::to_vec(&daemon).unwrap(); fs::write(&daemon_path, &bytes).unwrap();
    receipt.incarnation = Some(format!("pid={}:start={}:receipt={:x}", std::process::id(), process_start_identity(std::process::id() as i32).unwrap(), Sha256::digest(&bytes)));
    receipt.state = ReceiptState::Open;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); let port = listener.local_addr().unwrap().port();
    fs::write(Path::new(&receipt.profile_path).join("DevToolsActivePort"), format!("{port}\n/devtools/browser/fixture\n")).unwrap();
    receipt.cdp_endpoint = Some(format!("http://127.0.0.1:{port}")); receipt.cdp_browser_identity = Some("/devtools/browser/fixture".into()); receipt.target_id = Some("page-fixture".into()); f.service.store(receipt).unwrap();
    let cdp = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { break; }; let mut request = [0u8; 2048]; let n = stream.read(&mut request).await.unwrap();
            let body = if String::from_utf8_lossy(&request[..n]).contains("/json/version") { json!({"webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/browser/fixture")}) } else { json!([{"type":"page","id":"page-fixture"}]) };
            let body = serde_json::to_string(&body).unwrap(); let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()); stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let fresh = f.root.join(format!("fresh-daemon-{}.json", receipt.association_key)); let mut fresh_value = daemon; fresh_value["fixture_generation"] = json!("after"); fs::write(&fresh, serde_json::to_vec(&fresh_value).unwrap()).unwrap();
    let commands = f.root.join("live-commands"); fs::create_dir_all(&commands).unwrap();
    let script = format!("#!/bin/sh\ncase \"$2\" in\n cookie-get) echo undefined;;\n close) rm -- {}; echo closed;;\n open) cp -- {} {}; printf '%s' \"$3\" > {}; printf '{}\\n/devtools/browser/fixture\\n' > {}; echo '### Result';;\n *) exit 1;;\nesac\n", shell_quote(&daemon_path.display().to_string()), shell_quote(&fresh.display().to_string()), shell_quote(&daemon_path.display().to_string()), shell_quote(&f.root.join(format!("{}-url", receipt.association_key)).display().to_string()), port, shell_quote(&Path::new(&receipt.profile_path).join("DevToolsActivePort").display().to_string()));
    fs::write(commands.join(format!("{}.sh", receipt.association_key)), script).unwrap();
    fs::write(&f.service.configuration.playwright_cli, format!("#!/bin/sh\nif [ \"$1\" = --version ]; then echo 0.1.5; exit; fi\nkey=\"${{1#-s=cockpit-}}\"\nexec sh {}/\"$key.sh\" \"$@\"\n", shell_quote(&commands.display().to_string()))).unwrap();
    LiveFixture { daemon_dir, _socket: socket, cdp }
}

#[tokio::test]
async fn open_fresh_stops_live_incarnation_discards_cookies_and_starts_at_default_url() {
    let f = Fixture::new(); let mut a = f.receipt("w1:t1").await; let _live = seed_live(&f, &mut a).await;
    fs::write(Path::new(&a.profile_path).join("cookies"), b"old-login").unwrap();
    let previous = a.incarnation.clone();
    let response = f.service.execute(BrowserRequest { target: f.target("w1:t1"), action: BrowserAction::OpenFresh { url: None } }).await.unwrap();
    assert_eq!(response.connection, BrowserConnectionState::Open);
    assert_ne!(response.association.unwrap().incarnation, previous);
    assert!(!Path::new(&a.profile_path).join("cookies").exists());
    assert_eq!(fs::read_to_string(f.root.join(format!("{}-url", a.association_key))).unwrap(), "https://default.test/start");
}

#[tokio::test]
async fn legacy_cutover_stops_seeded_live_session_but_preserves_every_artifact() {
    let f = Fixture::new(); let mut a = f.receipt("w1:t1").await; let _live = seed_live(&f, &mut a).await;
    fs::write(Path::new(&a.profile_path).join("cookies"), b"legacy-login").unwrap(); let protected = f.protected(); f.legacy(&a);
    f.service.retire_legacy_space_associations().await.unwrap();
    assert!(f.service.inspect_live(&a).await.as_ref().is_err_and(may_launch_after_inspection_failure));
    assert_eq!(fs::read(Path::new(&a.profile_path).join("cookies")).unwrap(), b"legacy-login");
    assert!(Path::new(&a.working_directory).exists()); assert!(Path::new(&a.config_path).exists()); assert_protected(&protected);
    assert_eq!(f.service.legacy_list().await.unwrap().archives[0].session_stopped, true);
}

#[tokio::test]
async fn closing_one_of_two_live_tab_sessions_keeps_the_other_open_and_byte_identical() {
    let f = Fixture::new();
    let mut a = f.receipt("w1:t1").await; let _live_a = seed_live(&f, &mut a).await;
    let mut b = f.receipt("w1:t2").await; let _live_b = seed_live(&f, &mut b).await;
    fs::write(Path::new(&b.profile_path).join("cookies"), b"tab b login").unwrap();
    let before = fs::read(f.service.tab_association_path(&b.association_key)).unwrap();
    assert_eq!(f.close("w1:t1").await.cleanup, BrowserCleanupState::Done);
    assert_eq!(fs::read(f.service.tab_association_path(&b.association_key)).unwrap(), before);
    let status = f.service.execute(BrowserRequest { target: f.target("w1:t2"), action: BrowserAction::Status }).await.unwrap();
    assert_eq!(status.connection, BrowserConnectionState::Open);
    assert_eq!(status.association.unwrap().incarnation, b.incarnation);
    assert_eq!(fs::read(Path::new(&b.profile_path).join("cookies")).unwrap(), b"tab b login");
}
