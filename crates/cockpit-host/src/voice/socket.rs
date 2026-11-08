//! Narrow, same-user voice IPC. The origin is stamped by the trusted host
//! transport, never copied from a browser request. Same-UID processes are
//! within the local trust boundary; this protocol is not remote authentication.

use std::{future::Future, io, path::{Path, PathBuf}, sync::Arc, time::Duration};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, net::{UnixListener, UnixStream}, sync::Semaphore, task::{JoinHandle, JoinSet}};

pub const VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 128 * 1024;
const MAX_CONNECTIONS: usize = 32;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(210);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin { Native, Web }

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum Operation { Connect(Value), Events(Value), Control(Value) }

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request { pub version: u32, pub origin: Origin, pub operation: Operation }

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response { pub version: u32, pub result: Result<Value, String> }

fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
fn denied(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, message) }

pub(crate) fn socket_path(root: &Path) -> PathBuf { root.join("voice").join("owner.sock") }

pub(crate) fn verify_dir(path: &Path, private: bool) -> io::Result<()> {
    let m = std::fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != nix::unistd::geteuid().as_raw()
        || (private && m.mode() & 0o7777 != 0o700)
        || (!private && m.mode() & 0o022 != 0) {
        return Err(denied("voice directory is not safely owned and private"));
    }
    Ok(())
}

pub(crate) fn verify_socket(root: &Path) -> io::Result<()> {
    verify_dir(root, false)?;
    verify_dir(&root.join("voice"), true)?;
    let m = std::fs::symlink_metadata(socket_path(root))?;
    if !m.file_type().is_socket() || m.uid() != nix::unistd::geteuid().as_raw() || m.mode() & 0o7777 != 0o600 {
        return Err(denied("voice socket is not private and owned"));
    }
    Ok(())
}

pub(crate) fn verify_peer(stream: &UnixStream) -> io::Result<()> {
    if stream.peer_cred()?.uid() != nix::unistd::geteuid().as_raw() {
        return Err(denied("voice peer identity does not match"));
    }
    Ok(())
}

// A physical frame includes its newline. Do not let BufReader accumulate an
// unbounded line, and reject EOF without a terminator.
pub(crate) async fn read_frame(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(stream);
    let mut frame = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() { return Err(invalid("unterminated voice frame")); }
        let length = available.iter().position(|b| *b == b'\n').map_or(available.len(), |n| n + 1);
        if frame.len() + length > MAX_FRAME_BYTES { return Err(invalid("voice frame too large")); }
        let done = available[length - 1] == b'\n';
        frame.extend_from_slice(&available[..length]);
        reader.consume(length);
        if done { frame.pop(); return Ok(frame); }
    }
}

pub(crate) async fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    struct Bounded(Vec<u8>);
    impl io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_FRAME_BYTES - 1 - self.0.len() {
                return Err(invalid("voice frame too large"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let mut bytes = Bounded(Vec::new());
    serde_json::to_writer(&mut bytes, value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    bytes.0.push(b'\n');
    stream.write_all(&bytes.0).await
}

/// Owns exactly the socket inode it bound. Dropping it stops its listener and
/// removes that inode only if the pathname still identifies it.
pub struct Server { task: JoinHandle<()>, path: PathBuf, device: u64, inode: u64 }

impl Server {
    pub fn bind<F, Fut>(root: impl AsRef<Path>, callback: F) -> io::Result<Self>
    where F: Fn(Request) -> Fut + Send + Sync + 'static,
          Fut: Future<Output = Result<Value, String>> + Send + 'static {
        let root = root.as_ref();
        verify_dir(root, false)?;
        let directory = root.join("voice");
        match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {},
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
            Err(e) => return Err(e),
        }
        verify_dir(&directory, true)?;
        let path = socket_path(root);
        // Bind never unlinks an existing entry, including a stale socket.
        let listener = UnixListener::bind(&path)?;
        let initial = std::fs::symlink_metadata(&path)?;
        let identity = (initial.dev(), initial.ino());
        let setup = (|| {
            if !initial.file_type().is_socket() || initial.uid() != nix::unistd::geteuid().as_raw() {
                return Err(denied("unexpected voice socket"));
            }
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            verify_socket(root)?;
            let current = std::fs::symlink_metadata(&path)?;
            if (current.dev(), current.ino()) != identity { return Err(denied("voice socket changed during bind")); }
            Ok(())
        })();
        if let Err(e) = setup { remove_own(&path, identity.0, identity.1); return Err(e); }
        let callback = Arc::new(callback);
        let task = tokio::spawn(async move {
            let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    Some(_) = connections.join_next(), if !connections.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        if verify_peer(&stream).is_err() { continue; }
                        let Ok(permit) = permits.clone().try_acquire_owned() else { continue; };
                        let callback = callback.clone();
                        connections.spawn(async move {
                            let _permit = permit;
                            let _ = tokio::time::timeout(REQUEST_TIMEOUT, async {
                                let frame = read_frame(&mut stream).await?;
                                let request: Request = serde_json::from_slice(&frame).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                                if request.version != VERSION { return Err(invalid("unsupported voice protocol version")); }
                                let response = Response { version: VERSION, result: callback(request).await };
                                write_frame(&mut stream, &response).await
                            }).await;
                        });
                    }
                }
            }
        });
        Ok(Self { task, path, device: identity.0, inode: identity.1 })
    }

    pub async fn shutdown(mut self) { self.task.abort(); let _ = (&mut self.task).await; }
}

fn remove_own(path: &Path, device: u64, inode: u64) {
    if let Ok(m) = std::fs::symlink_metadata(path) {
        if m.file_type().is_socket() && m.uid() == nix::unistd::geteuid().as_raw() && m.dev() == device && m.ino() == inode {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) { self.task.abort(); remove_own(&self.path, self.device, self.inode); }
}
