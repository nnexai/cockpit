//! Loopback Confluence REST v1 fixture for running the real confluence-cli.
//!
//! Binds `127.0.0.1:0`, serves synthetic pages only, and writes a private
//! temporary CLI configuration (`authType: none`, `readOnly: true`). The
//! `confluence` wrapper it creates logs argv only, drops every inherited
//! `CONFLUENCE_*` variable except the two read-only settings Cockpit must
//! pass, and points `CONFLUENCE_CONFIG_DIR`, `HOME` and `NETRC` at the
//! temporary directory so no real profile, token or netrc is read.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub const PROFILE: &str = "cockpit-fake";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `forceCloud`, API under `/wiki/rest/api`, page URLs `/wiki/spaces/<K>/pages/<id>/<title>`.
    Cloud,
    /// API under `/rest/api`, page URLs `/display/<K>/<title>`.
    DataCenter,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub space_key: String,
    pub space_name: String,
    pub version: u64,
    /// Storage-format XHTML; the CLI converts it to Markdown.
    pub storage: String,
    /// `(id, type, title)`, root first.
    pub ancestors: Vec<(String, String, String)>,
    pub labels: Vec<String>,
    /// `(id, title, media type, bytes)`.
    pub attachments: Vec<(String, String, String, u64)>,
}

#[derive(Default)]
pub struct State {
    pub pages: BTreeMap<String, Page>,
    /// Every request line as `METHOD path?query`.
    pub requests: Vec<String>,
    /// Answer every request with HTTP 401.
    pub unauthorized: bool,
    /// Report this `_links.base` instead of the server's own origin.
    pub links_base: Option<String>,
}

pub struct FakeConfluence {
    pub mode: Mode,
    pub port: u16,
    pub root: PathBuf,
    pub state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeConfluence {
    pub fn start(mode: Mode, cli: &Path) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cockpit-fake-confluence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for directory in ["config", "home", "bin"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = state.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let state = state.clone();
                            std::thread::spawn(move || serve(stream, mode, port, &state));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            })
        };
        let api_path = match mode {
            Mode::Cloud => "/wiki/rest/api",
            Mode::DataCenter => "/rest/api",
        };
        let mut profile = json!({
            "domain": format!("127.0.0.1:{port}"),
            "protocol": "http",
            "apiPath": api_path,
            "authType": "none",
            "readOnly": true,
        });
        if mode == Mode::Cloud {
            profile["forceCloud"] = json!(true);
        }
        std::fs::write(
            root.join("config/config.json"),
            serde_json::to_vec_pretty(&json!({
                "activeProfile": PROFILE,
                "profiles": { PROFILE: profile },
            }))
            .unwrap(),
        )
        .unwrap();
        let wrapper = root.join("bin/confluence");
        std::fs::write(
            &wrapper,
            format!(
                r#"#!/usr/bin/env python3
import json, os, sys
with open({log:?}, 'a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\n')
if os.environ.get('CONFLUENCE_READ_ONLY') != 'true' or os.environ.get('CONFLUENCE_CLI_ANALYTICS') != 'false':
    sys.stderr.write('Cockpit did not pass the read-only environment\n')
    sys.exit(97)
env = {{key: value for key, value in os.environ.items() if not key.startswith('CONFLUENCE_')}}
env.update({{
    'CONFLUENCE_READ_ONLY': 'true',
    'CONFLUENCE_CLI_ANALYTICS': 'false',
    'CONFLUENCE_CONFIG_DIR': {config:?},
    'HOME': {home:?},
    'XDG_CONFIG_HOME': {home:?} + '/.config',
    'NETRC': {home:?} + '/netrc-absent',
}})
os.execve({cli:?}, [{cli:?}] + sys.argv[1:], env)
"#,
                log = root.join("argv.jsonl").to_str().unwrap(),
                config = root.join("config").to_str().unwrap(),
                home = root.join("home").to_str().unwrap(),
                cli = cli.to_str().unwrap(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            mode,
            port,
            root,
            state,
            stop,
            thread: Some(thread),
        }
    }

    /// The provider `base_url` for this instance.
    pub fn base_url(&self) -> String {
        match self.mode {
            Mode::Cloud => format!("http://127.0.0.1:{}/wiki", self.port),
            Mode::DataCenter => format!("http://127.0.0.1:{}", self.port),
        }
    }

    pub fn executable(&self) -> String {
        self.root.join("bin/confluence").to_str().unwrap().into()
    }

    pub fn add_page(&self, page: Page) {
        self.state
            .lock()
            .unwrap()
            .pages
            .insert(page.id.clone(), page);
    }

    pub fn update_page(&self, id: &str, change: impl FnOnce(&mut Page)) {
        change(self.state.lock().unwrap().pages.get_mut(id).unwrap());
    }

    /// Logged argv, one call per entry; never the environment.
    pub fn argv(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.root.join("argv.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Drop for FakeConfluence {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn web_title(title: &str) -> String {
    title
        .bytes()
        .map(|byte| match byte {
            b' ' => "+".to_owned(),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn page_json(page: &Page, mode: Mode, base: &str) -> Value {
    let webui = match mode {
        Mode::Cloud => format!(
            "/spaces/{}/pages/{}/{}",
            page.space_key,
            page.id,
            web_title(&page.title)
        ),
        Mode::DataCenter => format!("/display/{}/{}", page.space_key, web_title(&page.title)),
    };
    let by = json!({
        "type": "known",
        "accountId": "<account-id>",
        "email": "<email>",
        "displayName": "Fixture Author",
        "publicName": "Fixture Author",
    });
    json!({
        "id": page.id,
        "type": "page",
        "status": "current",
        "title": page.title,
        "space": { "key": page.space_key, "name": page.space_name, "type": "global" },
        "history": {
            "latest": true,
            "createdBy": by,
            "createdDate": "2026-09-01T08:00:00.000Z",
            "lastUpdated": { "by": by, "when": "2026-09-25T14:03:11.000Z", "number": page.version },
        },
        "version": { "by": by, "when": "2026-09-25T14:03:11.000Z", "number": page.version, "minorEdit": false },
        "position": page.ancestors.len() as i64,
        "ancestors": page.ancestors.iter().map(|(id, kind, title)| json!({
            "id": id, "type": kind, "status": "current", "title": title,
        })).collect::<Vec<_>>(),
        "body": { "storage": { "value": page.storage, "representation": "storage" } },
        "_links": {
            "base": base,
            "context": if mode == Mode::Cloud { "/wiki" } else { "" },
            "webui": webui,
            "self": format!("{base}/rest/api/content/{}", page.id),
        },
    })
}

/// Undo `escapeCql` for one double-quoted literal starting at `rest`.
fn cql_literal(rest: &str) -> Option<(String, &str)> {
    let mut value = String::new();
    let mut chars = rest.char_indices();
    while let Some((index, character)) = chars.next() {
        match character {
            '\\' => value.push(chars.next()?.1),
            '"' => return Some((value, &rest[index + 1..])),
            other => value.push(other),
        }
    }
    None
}

fn respond(stream: &mut TcpStream, status: u16, body: &Value) {
    let body = serde_json::to_vec(body).unwrap();
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

fn serve(mut stream: TcpStream, mode: Mode, port: u16, state: &Mutex<State>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => break,
            Ok(_) if header == "\r\n" => break,
            Ok(_) => {}
        }
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")).unwrap();
    let query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    let mut state = state.lock().unwrap();
    state.requests.push(format!("{method} {target}"));
    if state.unauthorized {
        respond(
            &mut stream,
            401,
            &json!({"code": 401, "message": "Unauthorized"}),
        );
        return;
    }
    let api = match mode {
        Mode::Cloud => "/wiki/rest/api/",
        Mode::DataCenter => "/rest/api/",
    };
    let base = state.links_base.clone().unwrap_or_else(|| match mode {
        Mode::Cloud => format!("http://127.0.0.1:{port}/wiki"),
        Mode::DataCenter => format!("http://127.0.0.1:{port}"),
    });
    let not_found = |id: &str| {
        json!({
            "statusCode": 404,
            "message": format!("com.atlassian.confluence.api.service.exceptions.api.NotFoundException: No content found with id : {id}"),
        })
    };
    let Some(path) = url.path().strip_prefix(api) else {
        respond(
            &mut stream,
            404,
            &json!({"statusCode": 404, "message": "unknown path"}),
        );
        return;
    };
    if method != "GET" {
        respond(
            &mut stream,
            400,
            &json!({"statusCode": 400, "message": "read-only fixture"}),
        );
        return;
    }
    let segments: Vec<&str> = path.split('/').collect();
    match segments.as_slice() {
        ["space"] => {
            let mut spaces = BTreeMap::new();
            for page in state.pages.values() {
                spaces.entry(page.space_key.clone()).or_insert_with(|| page.space_name.clone());
            }
            let results: Vec<_> = spaces.iter().map(|(key, name)| json!({
                "key": key, "name": name, "type": "global"
            })).collect();
            respond(&mut stream, 200, &json!({
                "results": results, "start": 0, "limit": 100, "size": results.len(),
                "_links": {"base": base, "context": if mode == Mode::Cloud { "/wiki" } else { "" }},
            }));
        }
        ["space", key] => {
            let page = state.pages.values().find(|page| page.space_key == *key);
            match page {
                Some(page) => {
                    let homepage = state.pages.values()
                        .find(|candidate| candidate.space_key == *key && candidate.ancestors.is_empty())
                        .unwrap_or(page);
                    respond(&mut stream, 200, &json!({
                        "key": key,
                        "name": page.space_name,
                        "homepage": {"id": homepage.id, "type": "page", "title": homepage.title},
                        "_links": {"base": base, "context": if mode == Mode::Cloud { "/wiki" } else { "" }},
                    }));
                }
                None => respond(&mut stream, 404, &not_found(key)),
            }
        }
        ["content", "search"] if query.get("cql").is_some_and(|cql| cql.contains("type=page")) => {
            let cql = query.get("cql").cloned().unwrap_or_default();
            let key = cql.strip_prefix("space=\"").and_then(|rest| rest.strip_suffix("\" and type=page"));
            let Some(key) = key else {
                respond(&mut stream, 400, &json!({"message":"invalid cql"}));
                return;
            };
            let all: Vec<_> = state.pages.values().filter(|page| page.space_key == key).collect();
            let start = query.get("start").or_else(|| query.get("cursor"))
                .and_then(|value| value.parse::<usize>().ok()).unwrap_or(0);
            let request_limit = query.get("limit").and_then(|value| value.parse::<usize>().ok()).unwrap_or(2);
            let limit = request_limit.min(2);
            let results: Vec<_> = all.iter().skip(start).take(limit)
                .map(|page| page_json(page, mode, &base)).collect();
            let next_start = start + results.len();
            let next = if next_start < all.len() {
                let param = if mode == Mode::Cloud { "cursor" } else { "start" };
                Some(format!("{}?cql={}&limit={request_limit}&expand=version%2Cancestors%2Cspace&{param}={next_start}",
                    if mode == Mode::Cloud { "/wiki/rest/api/content/search" } else { "/rest/api/content/search" },
                    url::form_urlencoded::byte_serialize(cql.as_bytes()).collect::<String>()))
            } else { None };
            respond(&mut stream, 200, &json!({
                "results": results, "start": start, "limit": request_limit, "size": results.len(),
                "totalSize": all.len(), "cqlQuery": cql,
                "_links": {"base": base, "context": if mode == Mode::Cloud { "/wiki" } else { "" }, "next": next},
            }));
        }
        ["search"] => {
            let cql = query.get("cql").cloned().unwrap_or_default();
            let found = cql
                .strip_prefix("title = \"")
                .and_then(cql_literal)
                .and_then(|(title, rest)| {
                    let (space, _) = cql_literal(rest.strip_prefix(" AND space = \"")?)?;
                    Some((title, space))
                })
                .and_then(|(title, space)| {
                    state
                        .pages
                        .values()
                        .find(|page| page.title == title && page.space_key == space)
                        .cloned()
                });
            let results = found
                .map(|page| {
                    vec![json!({"content": page_json(&page, mode, &base), "title": page.title})]
                })
                .unwrap_or_default();
            respond(
                &mut stream,
                200,
                &json!({"results": results, "start": 0, "limit": 1, "size": results.len()}),
            );
        }
        ["content", id] => match state.pages.get(*id) {
            Some(page) => respond(&mut stream, 200, &page_json(page, mode, &base)),
            None => respond(&mut stream, 404, &not_found(id)),
        },
        ["content", id, "label"] => match state.pages.get(*id) {
            Some(page) => {
                let results: Vec<_> = page
                    .labels
                    .iter()
                    .enumerate()
                    .map(|(index, name)| json!({"prefix": "global", "name": name, "id": format!("{}", 9000 + index)}))
                    .collect();
                respond(
                    &mut stream,
                    200,
                    &json!({"results": results, "start": 0, "limit": 200, "size": results.len()}),
                );
            }
            None => respond(&mut stream, 404, &not_found(id)),
        },
        ["content", id, "child", "attachment"] => match state.pages.get(*id) {
            Some(page) => {
                let results: Vec<_> = page
                    .attachments
                    .iter()
                    .map(|(attachment, title, media_type, bytes)| {
                        json!({
                            "id": attachment,
                            "type": "attachment",
                            "title": title,
                            "metadata": {"mediaType": media_type},
                            "extensions": {"mediaType": media_type, "fileSize": bytes},
                            "version": {"number": 1},
                            "_links": {"download": format!("/download/attachments/{id}/{}?version=1", web_title(title))},
                        })
                    })
                    .collect();
                respond(
                    &mut stream,
                    200,
                    &json!({"results": results, "start": 0, "limit": 50, "size": results.len()}),
                );
            }
            None => respond(&mut stream, 404, &not_found(id)),
        },
        _ => respond(
            &mut stream,
            404,
            &json!({"statusCode": 404, "message": "unknown endpoint"}),
        ),
    }
}
