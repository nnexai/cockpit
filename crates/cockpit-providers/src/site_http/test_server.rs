//! Local TCP fixture shared by transport and provider consumer tests.

use parking_lot::Mutex;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) authorization: Option<String>,
    pub(crate) accept: Option<String>,
}

pub(crate) enum Length {
    Exact,
    Declared(usize),
    Close,
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) location: Option<String>,
    pub(crate) body: Vec<u8>,
    pub(crate) length: Length,
    pub(crate) content_type: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
}

impl Reply {
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            location: None,
            body: Vec::new(),
            length: Length::Exact,
            content_type: None,
            headers: Vec::new(),
        }
    }
    pub(crate) fn json(value: serde_json::Value) -> Self {
        Self {
            body: serde_json::to_vec(&value).unwrap(),
            content_type: Some("application/json".into()),
            ..Self::status(200)
        }
    }
    pub(crate) fn bytes(body: &[u8]) -> Self {
        Self {
            body: body.to_vec(),
            ..Self::status(200)
        }
    }
    pub(crate) fn redirect(status: u16, location: &str) -> Self {
        Self {
            location: Some(location.into()),
            ..Self::status(status)
        }
    }
    pub(crate) fn length(mut self, length: Length) -> Self {
        self.length = length;
        self
    }
    pub(crate) fn content_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.into());
        self
    }
    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

pub(crate) struct Server {
    pub(crate) addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stopped: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(
        handler: impl Fn(&Seen, SocketAddr) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (log, stop, handler) = (seen.clone(), stopped.clone(), Arc::new(handler));
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let (handler, log) = (handler.clone(), log.clone());
                std::thread::spawn(move || {
                    let Ok(mut stream) = stream else {
                        return;
                    };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if head.len() > 64 * 1024 || stream.read(&mut byte).unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head);
                    let mut lines = head.split("\r\n");
                    let mut first = lines.next().unwrap().split(' ');
                    let method = first.next().unwrap().to_owned();
                    let path = first.next().unwrap().to_owned();
                    let mut request = Seen {
                        method,
                        path,
                        authorization: None,
                        accept: None,
                    };
                    for line in lines {
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("authorization") {
                                request.authorization = Some(value.trim().into());
                            }
                            if name.eq_ignore_ascii_case("accept") {
                                request.accept = Some(value.trim().into());
                            }
                        }
                    }
                    log.lock().push(request.clone());
                    let reply = handler(&request, addr);
                    let mut out = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
                    if let Some(location) = &reply.location {
                        out.push_str(&format!("Location: {location}\r\n"));
                    }
                    if let Some(content_type) = &reply.content_type {
                        out.push_str(&format!("Content-Type: {content_type}\r\n"));
                    }
                    for (name, value) in &reply.headers {
                        out.push_str(&format!("{name}: {value}\r\n"));
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
        Self {
            addr,
            seen,
            stopped,
            thread: Some(thread),
        }
    }
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen.lock().clone()
    }
    pub(crate) fn count(&self, prefix: &str) -> usize {
        self.seen()
            .iter()
            .filter(|seen| seen.path.starts_with(prefix))
            .count()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
