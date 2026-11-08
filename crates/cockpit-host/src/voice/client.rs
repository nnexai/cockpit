//! Single-request client for the private voice owner socket.
use std::{io, path::{Path, PathBuf}, time::Duration};
use serde_json::Value;
use tokio::net::UnixStream;
use super::socket::{self, Operation, Origin, Request, Response, VERSION};

/// The host chooses `origin` from its own transport context, not browser input.
#[derive(Clone)]
pub struct Client { root: PathBuf, origin: Origin }

impl Client {
    pub fn new(root: impl AsRef<Path>, origin: Origin) -> Self {
        Self { root: root.as_ref().to_owned(), origin }
    }

    pub async fn connect(&self, payload: Value) -> io::Result<Value> { self.request(Operation::Connect(payload)).await }
    pub async fn events(&self, payload: Value) -> io::Result<Value> { self.request(Operation::Events(payload)).await }
    pub async fn control(&self, payload: Value) -> io::Result<Value> { self.request(Operation::Control(payload)).await }

    async fn request(&self, operation: Operation) -> io::Result<Value> {
        tokio::time::timeout(Duration::from_secs(215), async {
            socket::verify_socket(&self.root)?;
            let mut stream = UnixStream::connect(socket::socket_path(&self.root)).await?;
            socket::verify_peer(&stream)?;
            socket::write_frame(&mut stream, &Request { version: VERSION, origin: self.origin, operation }).await?;
            let frame = socket::read_frame(&mut stream).await.map_err(|e| io::Error::new(e.kind(), format!("voice request outcome unknown: {e}")))?;
            let response: Response = serde_json::from_slice(&frame).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if response.version != VERSION { return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported voice protocol version")); }
            response.result.map_err(io::Error::other)
        }).await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "voice request timed out; outcome unknown"))?
    }
}
