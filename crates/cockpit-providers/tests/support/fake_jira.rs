use parking_lot::Mutex;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use cockpit_core::credentials::{MemoryVault, ProviderCredentials};
use cockpit_core::sources::{SourceFetchRequest, instance_authority};
use cockpit_protocol::credentials::{ProviderAuthKind, ProviderCredentialSetRequest};
use cockpit_protocol::projects::{
    ProjectConfiguration, ProjectLimits, ProjectProvider, ProviderDeployment, ProviderKind,
};
use cockpit_providers::credential_kinds;
use cockpit_providers::jira::JiraSourceProvider;
use serde_json::Value;
use url::Url;

pub const TOKEN: &str = "jira-http-fixture-token";
static NEXT: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
pub struct Request {
    pub url: Url,
    pub authenticated: bool,
}

impl Request {
    pub fn query(&self, name: &str) -> Option<String> {
        self.url
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }
}

pub struct FakeJira {
    pub base: String,
    pub provider: JiraSourceProvider,
    pub config: ProjectConfiguration,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    addr: SocketAddr,
    thread: Option<JoinHandle<()>>,
    root: PathBuf,
}

impl FakeJira {
    pub async fn new(
        deployment: ProviderDeployment,
        context: &str,
        stored: bool,
        respond: impl Fn(&Request, usize, &str) -> (u16, Value) + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{addr}");
        let base = format!("{origin}{context}");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let site = base.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => break,
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 1024];
                while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    let count = match stream.read(&mut buffer) {
                        Ok(count) => count,
                        Err(_) => 0,
                    };
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                if bytes.is_empty() {
                    continue;
                }
                let text = String::from_utf8(bytes).unwrap();
                let first = text.lines().next().unwrap();
                let target = first
                    .strip_prefix("GET ")
                    .unwrap()
                    .split(' ')
                    .next()
                    .unwrap();
                let authenticated = text.lines().any(|line| {
                    line.eq_ignore_ascii_case(&format!("authorization: Bearer {TOKEN}"))
                });
                let request = Request {
                    url: Url::parse(&format!("{origin}{target}")).unwrap(),
                    authenticated,
                };
                let index = {
                    let mut log = log.lock();
                    let index = log.len();
                    log.push(request.clone());
                    index
                };
                let (status, value) = respond(&request, index, &site);
                let body = serde_json::to_vec(&value).unwrap();
                let header = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream
                    .write_all(header.as_bytes())
                    .and_then(|_| stream.write_all(&body));
            }
        });
        let root = std::env::temp_dir().join(format!(
            "cockpit-jira-http-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let config = ProjectConfiguration {
            repository_roots: vec![],
            branch_template: "{repo}/{task_id}".into(),
            checkout_template: "{repo}-{task_id}".into(),
            providers: vec![ProjectProvider {
                id: "jira".into(),
                kind: ProviderKind::Jira,
                base_url: base.clone(),
                executable: None,
                login: None,
                deployment: Some(deployment),
            }],
            limits: ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 1,
                git_timeout_ms: 5000,
                git_output_bytes: 1024 * 1024,
                operation_timeout_ms: 2000,
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
            ..ProjectConfiguration::for_tests(&root)
        };
        let credentials = Arc::new(ProviderCredentials::new(
            &config,
            Arc::new(MemoryVault::default()),
            credential_kinds,
        ));
        if stored {
            credentials
                .set(ProviderCredentialSetRequest {
                    provider_id: "jira".into(),
                    kind: ProviderAuthKind::Bearer,
                    username: None,
                    token: TOKEN.into(),
                })
                .await
                .unwrap();
        }
        let provider =
            JiraSourceProvider::configured(&config, "jira", credentials.clone()).unwrap();
        Self {
            base,
            provider,
            config,
            requests,
            stop,
            addr,
            thread: Some(thread),
            root,
        }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().clone()
    }

    pub fn request(&self) -> SourceFetchRequest {
        let artifact_url = format!("{}/browse/OPS-7", self.base);
        SourceFetchRequest {
            provider_id: "jira".into(),
            authority: instance_authority(&self.config, "jira", &artifact_url).unwrap(),
            artifact_url,
        }
    }
}

impl Drop for FakeJira {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
