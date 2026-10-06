use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};

use cockpit_core::credentials::MemoryVault;
use cockpit_protocol::credentials::{ProviderAuthKind, ProviderCredentialSetRequest};
use cockpit_protocol::projects::{
    ProjectConfiguration, ProjectLimits, ProjectProvider, ProviderDeployment, ProviderKind,
};
use serde_json::json;

use super::test_server::{Length, Reply, Server};
use super::*;

const TOKEN: &str = "transport-token-private";
const MEDIA: &str = "media-query-private";

fn configuration(base_url: &str, service: Service) -> ProjectConfiguration {
    ProjectConfiguration {
        version: 1,
        orchestration: Default::default(),
        repository_roots: vec![],
        worktree_root: "/tmp/w".into(),
        companion_root: "/tmp/c".into(),
        state_root: "/tmp/s".into(),
        cache_root: "/tmp/k".into(),
        library_root: "/tmp/l".into(),
        notes_root: "/tmp/n".into(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: vec![ProjectProvider {
            id: "site".into(),
            kind: match service {
                Service::Jira => ProviderKind::Jira,
                Service::Confluence => ProviderKind::Confluence,
            },
            base_url: base_url.into(),
            executable: None,
            login: None,
            deployment: Some(ProviderDeployment::DataCenter),
        }],
        limits: ProjectLimits {
            catalog_depth: 1,
            catalog_entries: 1,
            git_timeout_ms: 5000,
            git_output_bytes: 1024,
            operation_timeout_ms: 20_000,
            context_preview_bytes: 1024,
            context_preview_lines: 100,
            context_directory_entries: 100,
            context_tree_depth: 4,
            library_folder_files: 512,
            library_folder_bytes: 32 * 1024 * 1024,
            library_file_bytes: 4 * 1024 * 1024,
            library_space_pages: 200,
            library_attachment_bytes: 25 * 1024 * 1024,
            library_item_attachment_bytes: 100 * 1024 * 1024,
            library_max_items: 20_000,
        },
        origins: Default::default(),
    }
}

async fn transport(
    base: &str,
    service: Service,
    auth: Option<ProviderAuthKind>,
    timeout: Duration,
) -> (SiteHttp, Arc<ProviderCredentials>) {
    let cfg = configuration(base, service);
    let credentials = Arc::new(ProviderCredentials::new(
        &cfg,
        Arc::new(MemoryVault::new()),
        crate::credential_kinds,
    ));
    if let Some(kind) = auth {
        credentials
            .set(ProviderCredentialSetRequest {
                provider_id: "site".into(),
                kind,
                username: (kind == ProviderAuthKind::Basic).then(|| "user".into()),
                token: TOKEN.into(),
            })
            .await
            .unwrap();
    }
    (
        SiteHttp::new(
            Url::parse(base).unwrap(),
            "site",
            service,
            timeout,
            credentials.clone(),
        ),
        credentials,
    )
}

async fn bearer(base: &str) -> SiteHttp {
    transport(
        base,
        Service::Jira,
        Some(ProviderAuthKind::Bearer),
        Duration::from_secs(2),
    )
    .await
    .0
}

fn private(failure: &HttpFailure) {
    let text = format!("{failure:?} {}", failure.error);
    for secret in [TOKEN, MEDIA, "127.0.0.1", "token=", "?"] {
        assert!(
            !text.contains(secret),
            "failure must not include request or response secrets"
        );
    }
}

#[tokio::test]
async fn json_headers_are_get_only_and_credentials_are_current() {
    for auth in [ProviderAuthKind::Bearer, ProviderAuthKind::Basic] {
        let server = Server::start(|_, _| Reply::json(json!({"ok": true})));
        let (http, credentials) = transport(
            &format!("http://{}/jira", server.addr),
            Service::Jira,
            Some(auth),
            Duration::from_secs(2),
        )
        .await;
        let url = http.endpoint(
            &["rest", "api", "2", "issue", "A-1"],
            &[("fields", "summary")],
        );
        assert_eq!(
            http.get_json(url.clone(), MAX_JSON_BYTES).await.unwrap(),
            json!({"ok": true})
        );
        let first = server.seen().pop().unwrap();
        assert_eq!(first.method, "GET");
        assert_eq!(first.accept.as_deref(), Some("application/json"));
        let expected = credentials.required("site").await.unwrap().authorization();
        assert_eq!(first.authorization.as_deref(), Some(expected.as_str()));
        credentials
            .set(ProviderCredentialSetRequest {
                provider_id: "site".into(),
                kind: ProviderAuthKind::Bearer,
                username: None,
                token: "replacement-token".into(),
            })
            .await
            .unwrap();
        http.get_json(url, MAX_JSON_BYTES).await.unwrap();
        assert_eq!(
            server.seen().last().unwrap().authorization.as_deref(),
            Some("Bearer replacement-token")
        );
    }
}

#[tokio::test]
async fn missing_token_sends_no_request() {
    let server = Server::start(|_, _| Reply::json(json!({})));
    let (http, _) = transport(
        &format!("http://{}", server.addr),
        Service::Jira,
        None,
        Duration::from_secs(2),
    )
    .await;
    let failure = http
        .get_json(http.endpoint(&["api"], &[]), MAX_JSON_BYTES)
        .await
        .unwrap_err();
    assert_eq!(failure.error.code, "source_credential_required");
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn endpoints_encode_segments_and_links_preserve_context() {
    let http = bearer("https://Jira.Example.test/jira/").await;
    let url = http.endpoint(&["rest", "../", "%2F", "a b"], &[("q", "a&b")]);
    assert_eq!(
        url.as_str(),
        "https://jira.example.test/jira/rest/..%2F/%252F/a%20b?q=a%26b"
    );
    assert!(http.on_site(&Url::parse("https://jira.example.test:443/jira/rest").unwrap()));
    for bad in [
        "https://jira.example.test/jira-other/rest",
        "http://jira.example.test/jira/rest",
        "https://user@jira.example.test/jira/rest",
        "https://jira.example.test:8443/jira/rest",
    ] {
        assert!(!http.on_site(&Url::parse(bad).unwrap()));
    }
    let http = bearer("https://wiki.example.test/wiki").await;
    for link in [
        "/download/file?id=1",
        "/wiki/download/file?id=1",
        "https://wiki.example.test/wiki/download/file?id=1",
    ] {
        assert_eq!(
            http.link(link).unwrap().as_str(),
            "https://wiki.example.test/wiki/download/file?id=1"
        );
    }
    for bad in [
        "https://other.example.test/wiki/download",
        "//other.example.test/wiki/download",
        "relative",
        "/../escape",
        "/wiki/../escape",
        "https://user:pw@wiki.example.test/wiki/download",
        "/\\other.example.test/x",
    ] {
        assert_eq!(http.link(bad).unwrap_err().kind, FailureKind::Contract);
    }
}

#[tokio::test]
async fn json_redirects_never_send_cross_site_or_outside_context_requests() {
    let foreign = Server::start(|_, _| Reply::json(json!({})));
    for location in [
        format!("http://{}/wiki/api", foreign.addr),
        "/outside/api".into(),
        "http://user:pw@127.0.0.1/wiki/api".into(),
    ] {
        let server = Server::start(move |_, _| Reply::redirect(302, &location));
        let http = bearer(&format!("http://{}/wiki", server.addr)).await;
        let failure = http
            .get_json(http.endpoint(&["api"], &[]), MAX_JSON_BYTES)
            .await
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Contract);
        private(&failure);
        assert_eq!(server.seen().len(), 1);
    }
    assert!(foreign.seen().is_empty());
}

#[tokio::test]
async fn json_redirect_hop_bound_and_missing_location() {
    for total in [3usize, 4] {
        let server = Server::start(move |request, _| {
            let hop = request
                .path
                .strip_prefix("/jira/")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            if hop < total {
                Reply::redirect(307, &format!("/jira/{}", hop + 1))
            } else {
                Reply::json(json!({"done": true}))
            }
        });
        let http = bearer(&format!("http://{}/jira", server.addr)).await;
        let result = http
            .get_json(http.endpoint(&["0"], &[]), MAX_JSON_BYTES)
            .await;
        if total == 3 {
            assert_eq!(result.unwrap(), json!({"done": true}));
        } else {
            assert_eq!(result.unwrap_err().kind, FailureKind::Contract);
        }
        assert_eq!(server.seen().len(), 4);
    }
    let server = Server::start(|_, _| Reply::status(302));
    let http = bearer(&format!("http://{}", server.addr)).await;
    assert_eq!(
        http.get_json(http.endpoint(&["api"], &[]), MAX_JSON_BYTES)
            .await
            .unwrap_err()
            .kind,
        FailureKind::Contract
    );
}

#[tokio::test]
async fn redirect_policy_refuses_downgrade_and_unsupported_targets() {
    let http = bearer("https://jira.example.test/jira").await;
    let current = Url::parse("https://jira.example.test/jira/a").unwrap();
    assert_eq!(
        http.next_hop(&current, "/jira/b").unwrap().as_str(),
        "https://jira.example.test/jira/b"
    );
    assert!(
        http.next_hop(&current, "https://media.example.test/file")
            .is_ok()
    );
    for link in [
        "http://jira.example.test/a",
        "ftp://jira.example.test/a",
        "file:///etc/passwd",
        "https://user@jira.example.test/a",
        "https://user:pw@jira.example.test/a",
        "http://[::",
    ] {
        let failure = http.next_hop(&current, link).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Contract);
        private(&failure);
    }
}

#[tokio::test]
async fn json_caps_content_type_and_parse_contract() {
    for (reply, kind, code) in [
        (
            Reply::json(json!({"data": "too long"})),
            FailureKind::TooLarge,
            "source_truncated",
        ),
        (
            Reply::json(json!({"data": "too long"})).length(Length::Close),
            FailureKind::TooLarge,
            "source_truncated",
        ),
        (
            Reply::bytes(b"{}").content_type("text/html"),
            FailureKind::Contract,
            "source_provider_contract",
        ),
        (
            Reply::bytes(b"{}"),
            FailureKind::Contract,
            "source_provider_contract",
        ),
        (
            Reply::bytes(b"bad").content_type("application/json"),
            FailureKind::Contract,
            "source_provider_contract",
        ),
    ] {
        let reply = parking_lot::Mutex::new(Some(reply));
        let server = Server::start(move |_, _| reply.lock().take().unwrap());
        let http = bearer(&format!("http://{}", server.addr)).await;
        let failure = http
            .get_json(http.endpoint(&["api"], &[]), 4)
            .await
            .unwrap_err();
        assert_eq!(failure.kind, kind);
        assert_eq!(failure.error.code, code);
        private(&failure);
    }
    for content_type in [
        "application/json; charset=utf-8",
        "application/problem+json",
    ] {
        let server = Server::start(move |_, _| Reply::json(json!({})).content_type(content_type));
        let http = bearer(&format!("http://{}", server.addr)).await;
        assert_eq!(
            http.get_json(http.endpoint(&["api"], &[]), 2)
                .await
                .unwrap(),
            json!({})
        );
    }
}

#[tokio::test]
async fn status_failures_have_fixed_private_codes_for_both_services() {
    for service in [Service::Jira, Service::Confluence] {
        for (status, kind, code) in [
            (400, FailureKind::Rejected, "source_provider_failed"),
            (401, FailureKind::Auth, "source_auth_failed"),
            (403, FailureKind::Auth, "source_auth_failed"),
            (404, FailureKind::NotFound, "source_not_found"),
            (410, FailureKind::NotFound, "source_not_found"),
            (429, FailureKind::RateLimited, "source_rate_limited"),
            (500, FailureKind::Status, "source_provider_failed"),
        ] {
            let server = Server::start(move |_, _| {
                let mut reply = Reply::bytes(TOKEN.as_bytes());
                reply.status = status;
                reply
            });
            let (http, _) = transport(
                &format!("http://{}", server.addr),
                service,
                Some(ProviderAuthKind::Bearer),
                Duration::from_secs(2),
            )
            .await;
            let failure = http
                .get_json(http.endpoint(&["api"], &[("token", MEDIA)]), MAX_JSON_BYTES)
                .await
                .unwrap_err();
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.status.map(|value| value.as_u16()), Some(status));
            assert_eq!(failure.error.code, code);
            assert!(failure.error.message.starts_with(service.name()));
            private(&failure);
        }
    }
}

#[tokio::test]
async fn timeout_and_closed_port_do_not_expose_urls_or_tokens() {
    let server = Server::start(|_, _| {
        std::thread::sleep(Duration::from_millis(200));
        Reply::json(json!({}))
    });
    let (http, _) = transport(
        &format!("http://{}", server.addr),
        Service::Jira,
        Some(ProviderAuthKind::Bearer),
        Duration::from_millis(20),
    )
    .await;
    let failure = http
        .get_json(http.endpoint(&["api"], &[("token", MEDIA)]), MAX_JSON_BYTES)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Timeout);
    assert_eq!(failure.status, None);
    assert_eq!(failure.error.code, "source_provider_timeout");
    private(&failure);
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let http = bearer(&format!("http://{address}")).await;
    let failure = http
        .get_json(http.endpoint(&["api"], &[("token", MEDIA)]), MAX_JSON_BYTES)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Unreachable);
    assert_eq!(failure.error.code, "source_provider_failed");
    private(&failure);
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Dest {
    path: std::path::PathBuf,
    dir: Dir,
}
impl Dest {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cockpit-site-http-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        Self { path, dir }
    }
    fn empty(&self) -> bool {
        std::fs::read_dir(&self.path).unwrap().next().is_none()
    }
}
impl Drop for Dest {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn downloads_drop_auth_on_foreign_hops_and_restore_it_on_return() {
    let site_address = Arc::new(parking_lot::Mutex::new(None));
    let return_address = site_address.clone();
    let media = Server::start(move |_, _| {
        Reply::redirect(
            302,
            &format!("http://{}/jira/final", return_address.lock().unwrap()),
        )
    });
    let media_address = media.addr;
    let site = Server::start(move |request, _| {
        if request.path == "/jira/final" {
            Reply::bytes(b"hello")
        } else {
            Reply::redirect(303, &format!("http://{media_address}/media?token={MEDIA}"))
        }
    });
    *site_address.lock() = Some(site.addr);
    let (http, credentials) = transport(
        &format!("http://{}/jira", site.addr),
        Service::Jira,
        Some(ProviderAuthKind::Basic),
        Duration::from_secs(2),
    )
    .await;
    let dest = Dest::new();
    http.download(http.endpoint(&["bytes"], &[]), 5, Some(5), &dest.dir)
        .await
        .unwrap();
    let authorization = credentials.required("site").await.unwrap().authorization();
    assert!(
        site.seen()
            .iter()
            .all(|seen| seen.authorization.as_deref() == Some(authorization.as_str()))
    );
    assert!(media.seen().iter().all(|seen| seen.authorization.is_none()));
    assert_eq!(
        std::fs::read(dest.path.join(DOWNLOADED_NAME)).unwrap(),
        b"hello"
    );
    assert_eq!(
        std::fs::metadata(dest.path.join(DOWNLOADED_NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[tokio::test]
async fn download_bounds_and_mismatches_remove_partial_files() {
    for (body, length, cap, expected) in [
        (b"hello".as_slice(), Length::Exact, 4, None),
        (b"hello".as_slice(), Length::Close, 4, None),
        (b"hello".as_slice(), Length::Exact, 10, Some(6)),
        (b"hello".as_slice(), Length::Close, 10, Some(6)),
        (b"hello".as_slice(), Length::Close, 10, Some(4)),
    ] {
        let reply = parking_lot::Mutex::new(Some(Reply::bytes(body).length(length)));
        let server = Server::start(move |_, _| reply.lock().take().unwrap());
        let http = bearer(&format!("http://{}", server.addr)).await;
        let dest = Dest::new();
        let failure = http
            .download(http.endpoint(&["bytes"], &[]), cap, expected, &dest.dir)
            .await
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::TooLarge);
        assert_eq!(failure.error.code, "source_attachment_size");
        assert!(dest.empty());
    }
}

#[tokio::test]
async fn download_never_follows_or_overwrites_existing_destination() {
    let server = Server::start(|_, _| Reply::bytes(b"changed"));
    let http = bearer(&format!("http://{}", server.addr)).await;
    for symlink in [false, true] {
        let dest = Dest::new();
        let original = dest.path.join("original");
        std::fs::write(&original, b"original").unwrap();
        if symlink {
            std::os::unix::fs::symlink("original", dest.path.join(DOWNLOADED_NAME)).unwrap();
        } else {
            std::fs::write(dest.path.join(DOWNLOADED_NAME), b"original").unwrap();
        }
        let failure = http
            .download(http.endpoint(&["bytes"], &[]), 100, None, &dest.dir)
            .await
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Write);
        assert_eq!(std::fs::read(&original).unwrap(), b"original");
        assert_eq!(
            std::fs::read(dest.path.join(DOWNLOADED_NAME)).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::symlink_metadata(dest.path.join(DOWNLOADED_NAME))
                .unwrap()
                .file_type()
                .is_symlink(),
            symlink
        );
    }
}

#[tokio::test]
async fn unsafe_initial_requests_and_oversized_records_send_nothing() {
    let site = Server::start(|_, _| Reply::json(json!({})));
    let foreign = Server::start(|_, _| Reply::json(json!({})));
    let http = bearer(&format!("http://{}/jira", site.addr)).await;
    for url in [
        format!("http://{}/jira/api", foreign.addr),
        format!("http://{}/outside/api", site.addr),
        format!("http://user:pw@{}/jira/api", site.addr),
    ] {
        let failure = http
            .get_json(Url::parse(&url).unwrap(), MAX_JSON_BYTES)
            .await
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Contract);
        private(&failure);
    }
    let dest = Dest::new();
    let failure = http
        .download(http.endpoint(&["bytes"], &[]), 4, Some(5), &dest.dir)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::TooLarge);
    assert!(dest.empty());
    assert!(site.seen().is_empty());
    assert!(foreign.seen().is_empty());
}
