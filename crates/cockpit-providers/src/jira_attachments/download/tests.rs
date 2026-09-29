use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cockpit_core::InspectionError;
use cockpit_core::credentials::{MemoryVault, ProviderCredentials};
use cockpit_core::process::StagingBudget;
use cockpit_core::sources::{AttachmentRef, SourceProvider};
use cockpit_protocol::credentials::{ProviderAuthKind, ProviderCredentialSetRequest};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
use serde_json::json;

use super::*;
use crate::credential_kinds;
use crate::jira::JiraSourceProvider;

const BEARER: &str = "pat-secret-91c4";
const API_TOKEN: &str = "api-token-1";
const MEDIA_SECRET: &str = "media-query-secret-5d2e";
/// base64 of `me@example.test:api-token-1`.
const BASIC: &str = "Basic bWVAZXhhbXBsZS50ZXN0OmFwaS10b2tlbi0x";

// ---- a loopback HTTP/1.1 server -------------------------------------------

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    authorization: Option<String>,
}

enum Length {
    /// `Content-Length` matches the body.
    Exact,
    /// `Content-Length` says this much, whatever the body holds.
    Declared(usize),
    /// No length: the body ends when the connection closes.
    Close,
}

struct Reply {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
    length: Length,
}

impl Reply {
    fn status(status: u16) -> Self {
        Self {
            status,
            location: None,
            body: Vec::new(),
            length: Length::Exact,
        }
    }
    fn json(value: serde_json::Value) -> Self {
        Self {
            body: serde_json::to_vec(&value).unwrap(),
            ..Self::status(200)
        }
    }
    fn bytes(body: &[u8]) -> Self {
        Self {
            body: body.to_vec(),
            ..Self::status(200)
        }
    }
    fn redirect(status: u16, location: &str) -> Self {
        Self {
            location: Some(location.into()),
            ..Self::status(status)
        }
    }
    fn length(mut self, length: Length) -> Self {
        self.length = length;
        self
    }
}

struct Server {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    /// `handler` gets the request and this server's own address.
    fn start(handler: impl Fn(&Seen, SocketAddr) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (handler, log) = (handler.clone(), log.clone());
                std::thread::spawn(move || {
                    let mut stream = stream.unwrap();
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if stream.read(&mut byte).unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let mut lines = head.split("\r\n");
                    let path = lines.next().unwrap().split(' ').nth(1).unwrap().to_owned();
                    let authorization = lines.find_map(|line| {
                        let (name, value) = line.split_once(": ")?;
                        name.eq_ignore_ascii_case("authorization")
                            .then(|| value.to_owned())
                    });
                    let request = Seen {
                        path,
                        authorization,
                    };
                    log.lock().unwrap().push(request.clone());
                    let reply = handler(&request, addr);
                    let mut out = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
                    if let Some(location) = &reply.location {
                        out.push_str(&format!("Location: {location}\r\n"));
                    }
                    match reply.length {
                        Length::Exact => {
                            out.push_str(&format!("Content-Length: {}\r\n", reply.body.len()))
                        }
                        Length::Declared(length) => {
                            out.push_str(&format!("Content-Length: {length}\r\n"))
                        }
                        Length::Close => {}
                    }
                    out.push_str("\r\n");
                    let _ = stream.write_all(out.as_bytes());
                    let _ = stream.write_all(&reply.body);
                });
            }
        });
        Self { addr, seen }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn count(&self, prefix: &str) -> usize {
        self.seen()
            .iter()
            .filter(|seen| seen.path.starts_with(prefix))
            .count()
    }
}

// ---- provider and destination fixtures -------------------------------------

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Dest {
    path: PathBuf,
    dir: Dir,
}

impl Dest {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cockpit-jira-download-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        Self { path, dir }
    }

    fn entries(&self) -> Vec<String> {
        std::fs::read_dir(&self.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }
}

impl Drop for Dest {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn configuration(base_url: &str) -> ProjectConfiguration {
    ProjectConfiguration {
        version: 1,
        repository_roots: vec![],
        worktree_root: "/tmp/w".into(),
        companion_root: "/tmp/c".into(),
        state_root: "/tmp/s".into(),
        cache_root: "/tmp/k".into(),
        library_root: "/tmp/l".into(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: vec![ProjectProvider {
            id: "jira".into(),
            base_url: base_url.into(),
            executable: "/nonexistent/jira".into(),
            login: None,
        }],
        limits: ProjectLimits {
            catalog_depth: 1,
            catalog_entries: 1,
            git_timeout_ms: 5000,
            git_output_bytes: 1024 * 1024,
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

enum Stored {
    Nothing,
    Bearer,
    Basic,
}

/// A provider for `base_url` with the given credential in a memory vault.
async fn jira_at(base_url: &str, stored: Stored) -> (JiraSourceProvider, Arc<MemoryVault>) {
    let configuration = configuration(base_url);
    let vault = Arc::new(MemoryVault::new());
    let credentials = Arc::new(ProviderCredentials::new(
        &configuration,
        vault.clone(),
        credential_kinds,
    ));
    let (kind, username, token) = match stored {
        Stored::Nothing => {
            let provider = JiraSourceProvider::configured(&configuration, "jira")
                .unwrap()
                .with_credentials(credentials);
            return (provider, vault);
        }
        Stored::Bearer => (ProviderAuthKind::Bearer, None, BEARER),
        Stored::Basic => (
            ProviderAuthKind::Basic,
            Some("me@example.test".into()),
            API_TOKEN,
        ),
    };
    credentials
        .set(ProviderCredentialSetRequest {
            provider_id: "jira".into(),
            kind,
            username,
            token: token.into(),
        })
        .await
        .unwrap();
    let provider = JiraSourceProvider::configured(&configuration, "jira")
        .unwrap()
        .with_credentials(credentials);
    (provider, vault)
}

fn attachment(bytes: Option<u64>) -> AttachmentRef {
    AttachmentRef {
        id: "10001".into(),
        title: "trace.log".into(),
        bytes,
    }
}

fn budget(bytes: u64) -> StagingBudget {
    StagingBudget {
        bytes,
        max_files: 1,
    }
}

async fn download(
    provider: &JiraSourceProvider,
    attachment: &AttachmentRef,
    dest: &Dest,
    bytes: u64,
) -> Result<cockpit_core::sources::DownloadedAttachment, InspectionError> {
    provider
        .download_attachment(
            "SCRUM-6",
            attachment,
            &[],
            &dest.dir,
            &dest.path,
            budget(bytes),
        )
        .await
}

/// No error may carry a credential, a URL query, or an address.
fn clean(error: &InspectionError) -> &InspectionError {
    let text = format!("{error:?} {error}");
    for secret in [BEARER, API_TOKEN, MEDIA_SECRET, "token=", "127.0.0.1", "?"] {
        assert!(!text.contains(secret), "error leaks {secret:?}: {text}");
    }
    error
}

fn refused(
    result: Result<cockpit_core::sources::DownloadedAttachment, InspectionError>,
    code: &str,
) {
    let error = result.expect_err("download must be refused");
    assert_eq!(error.code, code, "{error:?}");
    clean(&error);
}

fn metadata(content: &str, size: Option<u64>) -> Reply {
    let mut value = json!({"id": "10001", "filename": "trace.log", "content": content});
    if let Some(size) = size {
        value["size"] = json!(size);
    }
    Reply::json(value)
}

/// A site at `/jira` whose attachment 10001 is served at `serve(path)`.
fn site(size: Option<u64>, serve: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Server {
    Server::start(move |request, own| match request.path.as_str() {
        "/jira/rest/api/2/attachment/10001" => metadata(
            &format!("http://{own}/jira/secure/attachment/10001/trace.log"),
            size,
        ),
        path => serve(path),
    })
}

fn file(dest: &Dest) -> Vec<u8> {
    std::fs::read(dest.path.join("download")).unwrap()
}

// ---- Data Center and Cloud shapes -----------------------------------------

#[tokio::test]
async fn data_center_sends_bearer_to_both_requests_and_keeps_the_context_path() {
    let server = site(Some(5), |path| {
        if path == "/jira/secure/attachment/10001/trace.log" {
            Reply::bytes(b"hello")
        } else {
            Reply::status(404)
        }
    });
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();

    let done = download(&provider, &attachment(Some(5)), &dest, 1024)
        .await
        .unwrap();

    assert_eq!(done.attachment_id, "10001");
    assert_eq!(done.file_name, "download");
    assert_eq!(file(&dest), b"hello");
    let mode = std::fs::metadata(dest.path.join("download"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let seen = server.seen();
    assert_eq!(
        seen.iter()
            .map(|seen| seen.path.as_str())
            .collect::<Vec<_>>(),
        [
            "/jira/rest/api/2/attachment/10001",
            "/jira/secure/attachment/10001/trace.log"
        ]
    );
    let bearer = format!("Bearer {BEARER}");
    assert!(
        seen.iter()
            .all(|seen| seen.authorization.as_deref() == Some(bearer.as_str()))
    );
}

#[tokio::test]
async fn cloud_media_redirect_leaves_the_site_without_authorization() {
    let media = Server::start(|_, _| Reply::bytes(b"cloud bytes"));
    let media_addr = media.addr;
    let server = Server::start(move |request, own| match request.path.as_str() {
        "/rest/api/2/attachment/10001" => metadata(
            &format!("http://{own}/rest/api/3/attachment/content/10001"),
            Some(11),
        ),
        "/rest/api/3/attachment/content/10001" => Reply::redirect(
            303,
            &format!("http://{media_addr}/media/f?token={MEDIA_SECRET}"),
        ),
        _ => Reply::status(404),
    });
    let (provider, _vault) = jira_at(&format!("http://{}", server.addr), Stored::Basic).await;
    let dest = Dest::new();

    download(&provider, &attachment(Some(11)), &dest, 1024)
        .await
        .unwrap();

    assert_eq!(file(&dest), b"cloud bytes");
    let site = server.seen();
    assert_eq!(site.len(), 2);
    assert!(
        site.iter()
            .all(|seen| seen.authorization.as_deref() == Some(BASIC))
    );
    let foreign = media.seen();
    assert_eq!(foreign.len(), 1);
    assert!(foreign[0].path.contains(MEDIA_SECRET));
    assert_eq!(
        foreign[0].authorization, None,
        "the media host must not see the token"
    );
}

#[tokio::test]
async fn a_redirect_back_to_the_site_is_authorized_again() {
    let server = site(None, |path| match path {
        "/jira/secure/attachment/10001/trace.log" => Reply::redirect(302, "/jira/final"),
        "/jira/final" => Reply::bytes(b"ok"),
        _ => Reply::status(404),
    });
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();

    download(&provider, &attachment(None), &dest, 1024)
        .await
        .unwrap();

    let bearer = format!("Bearer {BEARER}");
    let last = server.seen().pop().unwrap();
    assert_eq!(last.path, "/jira/final");
    assert_eq!(last.authorization.as_deref(), Some(bearer.as_str()));
}

// ---- refused metadata and redirects ---------------------------------------

#[tokio::test]
async fn an_off_site_metadata_link_is_refused_without_a_second_request() {
    let foreign = Server::start(|_, _| Reply::bytes(b"x"));
    let foreign_addr = foreign.addr;
    let server = Server::start(move |request, own| match request.path.as_str() {
        "/jira/rest/api/2/attachment/10001" => metadata(
            &format!(
                "http://{foreign_addr}/jira/secure/attachment/10001/trace.log?token={MEDIA_SECRET}"
            ),
            None,
        ),
        "/jira/rest/api/2/attachment/10002" => {
            metadata(&format!("http://{own}/elsewhere/10002/trace.log"), None)
        }
        _ => Reply::status(404),
    });
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();

    refused(
        download(&provider, &attachment(None), &dest, 1024).await,
        "source_provider_contract",
    );
    assert!(foreign.seen().is_empty());
    assert_eq!(server.seen().len(), 1);

    // Same origin but outside the configured base path.
    let other = AttachmentRef {
        id: "10002".into(),
        ..attachment(None)
    };
    refused(
        download(&provider, &other, &dest, 1024).await,
        "source_provider_contract",
    );
    assert_eq!(server.seen().len(), 2);
    assert!(dest.entries().is_empty());
}

#[tokio::test]
async fn metadata_for_another_attachment_is_refused() {
    let server = Server::start(|_, own| {
        Reply::json(json!({"id": "99999", "content": format!("http://{own}/jira/x")}))
    });
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();

    refused(
        download(&provider, &attachment(None), &dest, 1024).await,
        "source_provider_contract",
    );
    assert_eq!(server.seen().len(), 1);
}

#[test]
fn hop_policy_refuses_downgrade_and_unsupported_targets() {
    let https = Url::parse("https://jira.example.test/jira/a").unwrap();
    let http = Url::parse("http://jira.example.test/jira/a").unwrap();

    assert_eq!(
        next_hop(&https, "/jira/b?x=1").unwrap().as_str(),
        "https://jira.example.test/jira/b?x=1"
    );
    assert_eq!(
        next_hop(&https, "https://media.example.test/f")
            .unwrap()
            .as_str(),
        "https://media.example.test/f"
    );
    assert_eq!(
        next_hop(&http, "https://jira.example.test/b")
            .unwrap()
            .scheme(),
        "https"
    );
    for refused in [
        "http://jira.example.test/b",
        "ftp://jira.example.test/b",
        "file:///etc/passwd",
        "https://user:pw@jira.example.test/b",
        "https://user@jira.example.test/b",
    ] {
        let error = next_hop(&https, refused).expect_err(refused);
        assert_eq!(error.code, "source_provider_contract");
        clean(&error);
    }
    assert!(next_hop(&https, "http://[::").is_err());
}

#[test]
fn origin_is_scheme_host_and_effective_port() {
    let site = Url::parse("https://Jira.Example.test/jira").unwrap();
    let same = |other: &str| same_origin(&Url::parse(other).unwrap(), &site);
    assert!(same("https://jira.example.test/x"));
    assert!(same("https://jira.example.test:443/x"));
    assert!(!same("https://jira.example.test:8443/x"));
    assert!(!same("http://jira.example.test/x"));
    assert!(!same("https://jira.example.test.evil.test/x"));
    assert!(!same("https://media.example.test/x"));
}

#[tokio::test]
async fn an_unsupported_redirect_scheme_is_refused() {
    let server = site(None, |path| match path {
        "/jira/secure/attachment/10001/trace.log" => {
            Reply::redirect(302, "ftp://127.0.0.1/x?token=1")
        }
        _ => Reply::status(404),
    });
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();

    refused(
        download(&provider, &attachment(None), &dest, 1024).await,
        "source_provider_contract",
    );
}

fn chain(total: usize) -> impl Fn(&str) -> Reply + Send + Sync + 'static {
    move |path| {
        if path == "/jira/secure/attachment/10001/trace.log" {
            return Reply::redirect(302, "/jira/r/1");
        }
        match path
            .strip_prefix("/jira/r/")
            .and_then(|n| n.parse::<usize>().ok())
        {
            Some(n) if n < total => Reply::redirect(301, &format!("/jira/r/{}", n + 1)),
            Some(_) => Reply::bytes(b"end"),
            None => Reply::status(404),
        }
    }
}

#[tokio::test]
async fn three_redirects_are_followed_and_a_fourth_is_refused() {
    // content -> r/1 -> r/2 -> r/3 (bytes): the content URL plus three hops.
    let server = site(None, chain(3));
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();
    download(&provider, &attachment(None), &dest, 1024)
        .await
        .unwrap();
    assert_eq!(file(&dest), b"end");

    let server = site(None, chain(4));
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();
    refused(
        download(&provider, &attachment(None), &dest, 1024).await,
        "source_provider_contract",
    );
    assert_eq!(server.count("/jira/secure/"), 1);
    assert_eq!(
        server.count("/jira/r/"),
        3,
        "no request is made past the hop limit"
    );
    assert!(dest.entries().is_empty());
}

#[tokio::test]
async fn a_redirect_without_a_location_is_refused() {
    let server = site(None, |_| Reply::status(302));
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();
    refused(
        download(&provider, &attachment(None), &dest, 1024).await,
        "source_provider_contract",
    );
}

// ---- status mapping -------------------------------------------------------

#[tokio::test]
async fn http_statuses_map_to_fixed_codes_on_both_requests() {
    for (status, code) in [
        (401, "source_auth_failed"),
        (403, "source_auth_failed"),
        (404, "source_not_found"),
        (500, "source_provider_failed"),
    ] {
        // Metadata request.
        let server = Server::start(move |_, _| Reply::status(status));
        let (provider, _vault) =
            jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
        let dest = Dest::new();
        refused(
            download(&provider, &attachment(None), &dest, 1024).await,
            code,
        );
        assert_eq!(server.seen().len(), 1);

        // Content request.
        let server = site(None, move |_| Reply::status(status));
        let (provider, _vault) =
            jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
        refused(
            download(&provider, &attachment(None), &dest, 1024).await,
            code,
        );
        assert_eq!(server.seen().len(), 2);
        assert!(dest.entries().is_empty());
    }
}

#[tokio::test]
async fn failures_after_a_cross_origin_hop_leak_neither_query_nor_address() {
    let media = Server::start(|_, _| Reply::status(500));
    let media_addr = media.addr;
    let server = site(None, move |_| {
        Reply::redirect(
            302,
            &format!("http://{media_addr}/media?token={MEDIA_SECRET}"),
        )
    });
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Basic).await;
    let dest = Dest::new();
    refused(
        download(&provider, &attachment(None), &dest, 1024).await,
        "source_provider_failed",
    );

    // A closed port: the connect error's Display would name the URL.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let closed_addr = closed.local_addr().unwrap();
    drop(closed);
    let server = site(None, move |_| {
        Reply::redirect(
            302,
            &format!("http://{closed_addr}/media?token={MEDIA_SECRET}"),
        )
    });
    let (provider, _vault) = jira_at(&format!("http://{}/jira", server.addr), Stored::Basic).await;
    refused(
        download(&provider, &attachment(None), &dest, 1024).await,
        "source_provider_failed",
    );
    assert!(dest.entries().is_empty());
}

// ---- sizes ----------------------------------------------------------------

#[tokio::test]
async fn sizes_are_enforced_by_header_by_stream_and_by_record() {
    let body = vec![b'x'; 5000];

    // Content-Length beyond the budget: refused before any byte is read.
    let server = site(None, |_| Reply::bytes(b"x").length(Length::Declared(1000)));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();
    refused(
        download(&p, &attachment(None), &dest, 10).await,
        "source_attachment_size",
    );
    assert!(dest.entries().is_empty());

    // No length, more bytes than the budget.
    let big = body.clone();
    let server = site(None, move |_| Reply::bytes(&big).length(Length::Close));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    refused(
        download(&p, &attachment(None), &dest, 100).await,
        "source_attachment_size",
    );
    assert!(dest.entries().is_empty(), "the partial file is removed");

    // No length, more bytes than Jira's record says.
    let big = body.clone();
    let server = site(Some(5), move |_| Reply::bytes(&big).length(Length::Close));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    refused(
        download(&p, &attachment(Some(5)), &dest, 1 << 20).await,
        "source_attachment_size",
    );
    assert!(dest.entries().is_empty());

    // Fewer bytes than Jira's record says.
    let server = site(Some(50), |_| Reply::bytes(b"short").length(Length::Close));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    refused(
        download(&p, &attachment(Some(50)), &dest, 1 << 20).await,
        "source_attachment_size",
    );
    assert!(dest.entries().is_empty());

    // Content-Length disagreeing with the record.
    let server = site(Some(5), |_| Reply::bytes(b"hello!").length(Length::Exact));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    refused(
        download(&p, &attachment(Some(5)), &dest, 1 << 20).await,
        "source_attachment_size",
    );
    assert!(dest.entries().is_empty());

    // The record itself is over the budget, or disagrees with the listing:
    // no content request is made.
    let server = site(Some(2000), |_| Reply::bytes(b"never"));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    refused(
        download(&p, &attachment(Some(2000)), &dest, 1000).await,
        "source_attachment_size",
    );
    refused(
        download(&p, &attachment(Some(7)), &dest, 1 << 20).await,
        "source_attachment_size",
    );
    assert_eq!(server.count("/jira/secure/"), 0);
    assert!(dest.entries().is_empty());
}

#[tokio::test]
async fn a_connection_cut_mid_body_leaves_no_partial_file() {
    let server = site(None, |_| {
        Reply::bytes(b"only ten b").length(Length::Declared(100))
    });
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();
    refused(
        download(&p, &attachment(None), &dest, 1 << 20).await,
        "source_provider_failed",
    );
    assert!(dest.entries().is_empty());
}

// ---- credentials and inputs -----------------------------------------------

#[tokio::test]
async fn without_a_stored_token_nothing_is_requested() {
    let server = site(None, |_| Reply::bytes(b"x"));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Nothing).await;
    let dest = Dest::new();

    refused(
        download(&p, &attachment(None), &dest, 1024).await,
        "source_credential_required",
    );
    let error = p.attachment_downloads("issue").await.unwrap_err();
    assert_eq!(error.code, "source_credential_required");
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn attachment_downloads_follow_the_credential_state() {
    let (p, vault) = jira_at("https://jira.example.test", Stored::Bearer).await;
    p.attachment_downloads("issue").await.unwrap();
    assert_eq!(
        p.attachment_downloads("page").await.unwrap_err().code,
        "source_capability_unavailable"
    );

    // A vault that fails before anything is cached.
    let (p, vault_down) = jira_at("https://jira.example.test", Stored::Nothing).await;
    vault_down.set_failing(true);
    assert_eq!(
        p.attachment_downloads("issue").await.unwrap_err().code,
        "credential_vault_unavailable"
    );
    vault_down.set_failing(false);
    assert_eq!(
        p.attachment_downloads("issue").await.unwrap_err().code,
        "source_credential_required"
    );
    drop(vault);
}

#[tokio::test]
async fn unsafe_ids_are_refused_before_any_request() {
    let server = site(None, |_| Reply::bytes(b"x"));
    let (p, _v) = jira_at(&format!("http://{}/jira", server.addr), Stored::Bearer).await;
    let dest = Dest::new();

    let traversal = AttachmentRef {
        id: "../1".into(),
        ..attachment(None)
    };
    refused(
        download(&p, &traversal, &dest, 1024).await,
        "source_provider_contract",
    );
    let error = p
        .download_attachment(
            "not-a-key",
            &attachment(None),
            &[],
            &dest.dir,
            Path::new("/"),
            budget(1024),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "source_provider_contract");
    assert!(server.seen().is_empty());
}
