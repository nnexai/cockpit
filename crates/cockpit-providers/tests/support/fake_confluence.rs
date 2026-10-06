//! Synthetic Cloud v2 / Data Center v1 fixture. No CLI, profiles or ambient auth.
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

pub const TOKEN: &str = "cockpit-confluence-http-fixture-token";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Cloud,
    DataCenter,
}
impl Mode {
    pub fn context(self) -> &'static str {
        match self {
            Self::Cloud => "/wiki",
            Self::DataCenter => "/confluence",
        }
    }
    pub fn api(self) -> &'static str {
        match self {
            Self::Cloud => "/wiki/api/v2",
            Self::DataCenter => "/confluence/rest/api",
        }
    }
}
#[derive(Clone)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub version: u64,
    pub storage: String,
    /// Root-to-parent `(id, type, title)`.
    pub ancestors: Vec<(String, String, String)>,
    pub labels: Vec<String>,
    pub attachments: Vec<(String, String, String, u64)>,
}
impl Page {
    pub fn new(id: &str, title: &str, ancestors: &[(&str, &str, &str)]) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            version: 7,
            storage: "<h1>Overview</h1><p>Hello <strong>world</strong>.</p>".into(),
            ancestors: ancestors
                .iter()
                .map(|(id, kind, title)| (id.to_string(), kind.to_string(), title.to_string()))
                .collect(),
            labels: vec!["release".into(), "engineering".into()],
            attachments: vec![(
                "att557057".into(),
                "release-flow.png".into(),
                "image/png".into(),
                4,
            )],
        }
    }
}
#[derive(Clone)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: &'static str,
    pub location: Option<String>,
    pub declared_length: Option<usize>,
}
impl Response {
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(&value).unwrap(),
            content_type: "application/json",
            location: None,
            declared_length: None,
        }
    }
    pub fn bytes(bytes: &[u8]) -> Self {
        Self {
            status: 200,
            body: bytes.to_vec(),
            content_type: "application/octet-stream",
            location: None,
            declared_length: None,
        }
    }
    pub fn redirect(location: String) -> Self {
        Self {
            status: 302,
            body: Vec::new(),
            content_type: "application/octet-stream",
            location: Some(location),
            declared_length: None,
        }
    }
}
#[derive(Clone)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub authorized: bool,
    pub authorization_present: bool,
}
pub struct State {
    pub pages: BTreeMap<String, Page>,
    pub space_keys: Vec<String>,
    pub requests: Vec<Request>,
    pub unauthorized: bool,
    pub user_status: u16,
    pub page_size: usize,
    pub next_override: Option<String>,
    pub overrides: BTreeMap<String, Response>,
    pub download_redirect: Option<String>,
    pub download_body: Vec<u8>,
    pub cancel_after_path: Option<(String, Arc<AtomicBool>)>,
    pub labels_unbounded: bool,
    pub attachments_unbounded: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            pages: BTreeMap::new(),
            space_keys: vec!["ENG".into()],
            requests: Vec::new(),
            unauthorized: false,
            user_status: 200,
            page_size: 2,
            next_override: None,
            overrides: BTreeMap::new(),
            download_redirect: None,
            download_body: vec![1, 2, 3, 4],
            cancel_after_path: None,
            labels_unbounded: false,
            attachments_unbounded: false,
        }
    }
}
pub struct FakeConfluence {
    pub mode: Mode,
    pub port: u16,
    pub state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl FakeConfluence {
    pub fn start(mode: Mode) -> Self {
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
                        Err(_) => std::thread::sleep(Duration::from_millis(2)),
                    }
                }
            })
        };
        Self {
            mode,
            port,
            state,
            stop,
            thread: Some(thread),
        }
    }
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, self.mode.context())
    }
    pub fn add_page(&self, page: Page) {
        self.state.lock().pages.insert(page.id.clone(), page);
    }
    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().requests.clone()
    }
    pub fn count(&self, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.target.split('?').next() == Some(path))
            .count()
    }
}
impl Drop for FakeConfluence {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn web_title(title: &str) -> String {
    url::form_urlencoded::byte_serialize(title.as_bytes())
        .collect::<String>()
        .replace("%20", "+")
}
pub fn page_json(page: &Page, mode: Mode) -> Value {
    let webui = if mode == Mode::Cloud {
        format!("/spaces/ENG/pages/{}/{}", page.id, web_title(&page.title))
    } else {
        format!("/display/ENG/{}", web_title(&page.title))
    };
    let parent = page.ancestors.last();
    if mode == Mode::Cloud {
        json!({"id":page.id,"title":page.title,"spaceId":"99","status":"current",
            "parentId":parent.map(|(id,_,_)|id),"parentType":parent.map(|(_,kind,_)|kind),"position":1,
            "version":{"number":page.version,"createdAt":"2026-09-25T14:03:11.000Z","authorId":"private-account-id"},
            "body":{"storage":{"value":page.storage,"representation":"storage"}},"_links":{"webui":webui}})
    } else {
        json!({"id":page.id,"type":"page","title":page.title,"status":"current","space":{"key":"ENG","name":"Engineering"},
            "position":1,"version":{"number":page.version,"when":"2026-09-25T14:03:11.000Z","by":{"displayName":"Fixture Author"}},
            "history":{"lastUpdated":{"when":"2026-09-25T14:03:11.000Z","by":{"displayName":"Fixture Author"}}},
            "ancestors":page.ancestors.iter().map(|(id,kind,title)|json!({"id":id,"type":kind,"title":title})).collect::<Vec<_>>(),
            "body":{"storage":{"value":page.storage,"representation":"storage"}},"_links":{"webui":webui}})
    }
}
fn space(mode: Mode, key: &str) -> Value {
    if mode == Mode::Cloud {
        json!({"id":"99","key":key,"name":"Engineering","homepageId":"10"})
    } else {
        json!({"id":"99","key":key,"name":"Engineering","homepage":{"id":"10"}})
    }
}
fn attachment_json(page: &Page, attachment: &(String, String, String, u64), mode: Mode) -> Value {
    let (id, title, media, size) = attachment;
    let download = format!(
        "/rest/api/content/{}/child/attachment/{id}/download",
        page.id
    );
    if mode == Mode::Cloud {
        json!({"id":id,"title":title,"pageId":page.id,"mediaType":media,"fileSize":size,"version":{"number":2},"downloadLink":download})
    } else {
        json!({"id":id,"type":"attachment","title":title,"container":{"id":page.id},"metadata":{"mediaType":media},"extensions":{"fileSize":size},"version":{"number":2},"_links":{"download":format!("/confluence{download}")}})
    }
}
fn paged(
    rows: Vec<Value>,
    mode: Mode,
    url: &url::Url,
    query: &BTreeMap<String, String>,
    state: &State,
    size: usize,
) -> Response {
    let start = query
        .get(if mode == Mode::Cloud {
            "cursor"
        } else {
            "start"
        })
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    let stop = (start + size).min(rows.len());
    let mut value = json!({"results": rows[start..stop], "size": stop-start});
    if mode == Mode::DataCenter {
        value["totalSize"] = json!(rows.len());
    }
    if stop < rows.len() {
        let mut next = url.clone();
        next.set_query(None);
        for (key, val) in query {
            if key != "start" && key != "cursor" {
                next.query_pairs_mut().append_pair(key, val);
            }
        }
        next.query_pairs_mut().append_pair(
            if mode == Mode::Cloud {
                "cursor"
            } else {
                "start"
            },
            &stop.to_string(),
        );
        let cloud_cql = mode == Mode::Cloud
            && url.path() == "/wiki/rest/api/content/search";
        if cloud_cql {
            next.query_pairs_mut().append_pair("next", "true").append_pair("start", &stop.to_string());
        }
        let path = if cloud_cql {
            next.path().strip_prefix("/wiki").unwrap()
        } else {
            next.path()
        };
        value["_links"] = json!({"next":state.next_override.clone().unwrap_or_else(|| format!("{path}?{}",next.query().unwrap()))});
    }
    Response::json(200, value)
}
fn route(mode: Mode, url: &url::Url, state: &State) -> Response {
    let path = url.path();
    let query = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
    if let Some(response) = state
        .overrides
        .get(url.as_str())
        .or_else(|| state.overrides.get(path))
    {
        return response.clone();
    }
    if state.unauthorized {
        return Response::json(401, json!({"message":"Unauthorized"}));
    }
    if path == format!("{}/rest/api/user", mode.context()) {
        return Response::json(
            state.user_status,
            json!({"displayName":"Fixture Author","accountId":"private-account-id"}),
        );
    }
    let download_path = format!("{}/rest/api/content/", mode.context());
    if path.starts_with(&download_path) && path.ends_with("/download") {
        return state
            .download_redirect
            .clone()
            .map(Response::redirect)
            .unwrap_or_else(|| Response::bytes(&state.download_body));
    }
    if path == format!("{}/rest/api/content/search", mode.context()) {
        let cql = query.get("cql").map(String::as_str).unwrap_or_default();
        let ids = cql.split_once("id IN (")
            .and_then(|(_, tail)| tail.split_once(')'))
            .map(|(ids, _)| ids.split(',').map(str::trim).collect::<Vec<_>>());
        let rows = state.pages.values()
            .filter(|page| ids.as_ref().is_none_or(|ids| ids.contains(&page.id.as_str())))
            .map(|page| page_json(page, Mode::DataCenter))
            .collect();
        return paged(rows, mode, url, &query, state, state.page_size);
    }
    let Some(relative) = path
        .strip_prefix(mode.api())
        .and_then(|path| path.strip_prefix('/'))
    else {
        return Response::json(404, json!({}));
    };
    let parts = relative.split('/').collect::<Vec<_>>();
    let rows = match parts.as_slice() {
        ["spaces"] | ["space"] => {
            let rows = state
                .space_keys
                .iter()
                .filter(|key| query.get("keys").is_none_or(|wanted| wanted == *key))
                .map(|key| space(mode, key))
                .collect();
            return paged(rows, mode, url, &query, state, state.page_size);
        }
        ["spaces", "99"] | ["space", "ENG"] => return Response::json(200, space(mode, "ENG")),
        ["folders", id] => {
            let found = state
                .pages
                .values()
                .flat_map(|page| page.ancestors.iter())
                .find(|(aid, kind, _)| aid == id && kind == "folder");
            return found
                .map(|(id, _, title)| Response::json(200, json!({"id":id,"title":title})))
                .unwrap_or_else(|| Response::json(404, json!({})));
        }
        ["spaces", "99", "pages"] | ["content", "search"] => state
            .pages
            .values()
            .map(|page| page_json(page, mode))
            .collect(),
        ["pages", id] | ["content", id] => {
            if let Some(page) = state.pages.get(*id) {
                return Response::json(200, page_json(page, mode));
            }
            for page in state.pages.values() {
                for attachment in &page.attachments {
                    if attachment.0 == *id {
                        return Response::json(200, attachment_json(page, attachment, mode));
                    }
                }
            }
            return Response::json(404, json!({}));
        }
        ["attachments", id] => {
            for page in state.pages.values() {
                for attachment in &page.attachments {
                    if attachment.0 == *id {
                        return Response::json(200, attachment_json(page, attachment, mode));
                    }
                }
            }
            return Response::json(404, json!({}));
        }
        ["pages", id, "ancestors"] => {
            let Some(page) = state.pages.get(*id) else {
                return Response::json(404, json!({}));
            };
            let rows = page
                .ancestors
                .iter()
                .map(|(id, kind, _)| json!({"id":id,"type":kind}))
                .collect();
            return paged(rows, mode, url, &query, state, 250);
        }
        ["pages", id, "labels"] | ["content", id, "label"] => {
            let Some(page) = state.pages.get(*id) else {
                return Response::json(404, json!({}));
            };
            let rows = page
                .labels
                .iter()
                .map(|name| json!({"name":name}))
                .collect::<Vec<_>>();
            let count = if state.labels_unbounded {
                rows.len()
            } else {
                query.get("limit").unwrap().parse().unwrap()
            };
            return paged(rows, mode, url, &query, state, count);
        }
        ["pages", id, "attachments"] | ["content", id, "child", "attachment"] => {
            let Some(page) = state.pages.get(*id) else {
                return Response::json(404, json!({}));
            };
            let rows = page
                .attachments
                .iter()
                .map(|att| attachment_json(page, att, mode))
                .collect::<Vec<_>>();
            let count = if state.attachments_unbounded {
                rows.len()
            } else {
                query.get("limit").unwrap().parse().unwrap()
            };
            return paged(rows, mode, url, &query, state, count);
        }
        ["pages"] | ["content"] => state
            .pages
            .values()
            .filter(|page| {
                query.get("title").is_none_or(|title| &page.title == title)
                    && query
                        .get("id")
                        .is_none_or(|ids| ids.split(',').any(|id| id == page.id))
            })
            .map(|page| page_json(page, mode))
            .collect::<Vec<_>>(),
        _ => return Response::json(404, json!({})),
    };
    let size = if query.contains_key("id") || query.contains_key("title") {
        query
            .get("limit")
            .and_then(|v| v.parse().ok())
            .unwrap_or(250)
    } else {
        state.page_size
    };
    paged(rows, mode, url, &query, state, size)
}
fn serve(mut stream: TcpStream, mode: Mode, port: u16, state: &Mutex<State>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() || first.is_empty() {
        return;
    }
    let mut authorization = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("authorization") {
                authorization = Some(value.trim().to_string());
            }
        }
    }
    let mut words = first.split_whitespace();
    let method = words.next().unwrap().to_string();
    let target = words.next().unwrap().to_string();
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")).unwrap();
    let response = {
        let mut state = state.lock();
        state.requests.push(Request {
            method,
            target,
            authorized: authorization.as_deref() == Some(&format!("Bearer {TOKEN}")),
            authorization_present: authorization.is_some(),
        });
        if let Some((path, cancel)) = &state.cancel_after_path {
            if path == url.path() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        route(mode, &url, &state)
    };
    let reason = match response.status {
        200 => "OK",
        302 => "Found",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let mut headers = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.content_type,
        response.declared_length.unwrap_or(response.body.len())
    );
    if let Some(location) = response.location {
        headers.push_str(&format!("Location: {location}\r\n"));
    }
    headers.push_str("\r\n");
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(&response.body);
    let _ = stream.flush();
}
