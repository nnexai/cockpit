//! Private, isolated OMP RPC transport for one caller-owned live voice session.
//!
//! This deliberately stays on protocol v1: every physical frame is bounded and
//! oversized upstream responses fail rather than being silently reassembled.
//! The owner must stop the peer's audio and call `live_stop` on disconnect;
//! dropping or shutting down this bridge also kills its own OMP child. A timed
//! out or cancelled command may already have been applied by OMP.

use std::{
    collections::HashMap,
    ffi::OsString,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, oneshot, Semaphore},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const MAX_FRAME: usize = 1024 * 1024;
const QUEUE_CAPACITY: usize = 16;
const MAX_PENDING: usize = 32;
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// HOME, PATH, LANG, TMPDIR and an optional OMP agent directory are supplied
/// explicitly by the trusted owner; other ambient variables are not inherited.
/// OMP's native-cache and session-directory overrides are rooted in `cwd`.
/// `cwd` must be a private, isolated directory managed by the owner, not a
/// supervisor checkout. The shared agent DB may receive normal authentication
/// refresh and bookkeeping writes, and other agent-directory state may change.
pub struct VoiceRpcConfig {
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub home: OsString,
    pub agent_dir: Option<OsString>,
    pub path: Option<OsString>,
    pub lang: Option<OsString>,
    pub tmpdir: Option<OsString>,
    pub startup_timeout: Duration,
    pub command_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceRpcError {
    #[error("invalid RPC configuration")]
    InvalidConfig,
    #[error("could not spawn RPC child: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("RPC transport closed")]
    Closed,
    #[error("RPC operation timed out; its outcome may be unknown")]
    Timeout,
    #[error("invalid or oversized RPC frame")]
    Protocol,
    #[error("RPC command failed: {0}")]
    Remote(String),
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

// Removal on future cancellation or timeout also bounds stale response state.
struct PendingGuard {
    id: String,
    pending: Pending,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

/// Owns exactly one RPC process. Only the events listed by the live/host-tool
/// protocol are forwarded; the receiver must be continuously drained. A full or
/// dropped event receiver closes the transport rather than losing a tool call or
/// terminal voice event. No audio, transcript, command, or stderr is logged.
pub struct VoiceRpc {
    writer: mpsc::Sender<Vec<u8>>,
    pending: Pending,
    slots: Arc<Semaphore>,
    next_id: AtomicU64,
    events: Option<mpsc::Receiver<Value>>,
    cancel: CancellationToken,
    owner: Option<JoinHandle<()>>,
    command_timeout: Duration,
    ready: Value,
}

impl VoiceRpc {
    /// Spawn the explicitly configured executable without a shell, await its
    /// initial ready frame, and return an owned bridge. The caller is responsible
    /// for ensuring executable and directory provenance and directory isolation.
    pub async fn spawn(config: VoiceRpcConfig) -> Result<Self, VoiceRpcError> {
        if !config.executable.is_absolute() || !config.cwd.is_absolute()
            || config.home.is_empty() || config.startup_timeout.is_zero()
            || config.command_timeout.is_zero()
        {
            return Err(VoiceRpcError::InvalidConfig);
        }
        let mut command = Command::new(&config.executable);
        command.args([
            "--mode", "rpc", "--no-session", "--no-tools", "--no-extensions",
            "--no-skills", "--no-rules", "--no-context-files",
        ]).current_dir(&config.cwd)
            .env_clear().env("HOME", &config.home)
            // Isolate writable native and session state without redirecting auth.
            .env("PI_NATIVES_DIR", config.cwd.join("natives"))
            .env("PI_CODING_AGENT_SESSION_DIR", config.cwd.join("sessions"))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(agent_dir) = &config.agent_dir {
            command.env("PI_CODING_AGENT_DIR", agent_dir);
        }
        for (name, value) in [("PATH", &config.path), ("LANG", &config.lang), ("TMPDIR", &config.tmpdir)] {
            if let Some(value) = value { command.env(name, value); }
        }
        let mut child = command.spawn().map_err(VoiceRpcError::Spawn)?;
        let stdin = child.stdin.take().ok_or(VoiceRpcError::Closed)?;
        let stdout = child.stdout.take().ok_or(VoiceRpcError::Closed)?;
        let (writer, mut writes) = mpsc::channel::<Vec<u8>>(QUEUE_CAPACITY);
        let (event_tx, events) = mpsc::channel(QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = oneshot::channel();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let reader_pending = pending.clone();
        let owner = tokio::spawn(async move {
            let reader_cancel = worker_cancel.clone();
            let mut reader = tokio::spawn(async move {
                read_frames(stdout, ready_tx, event_tx, reader_pending, reader_cancel).await
            });
            let writer_cancel = worker_cancel.clone();
            let mut writer_task = tokio::spawn(async move {
                let mut stdin = stdin;
                loop {
                    let frame = tokio::select! {
                        _ = writer_cancel.cancelled() => break,
                        frame = writes.recv() => match frame { Some(frame) => frame, None => break },
                    };
                    match timeout(WRITE_TIMEOUT, stdin.write_all(&frame)).await {
                        Ok(Ok(())) => {},
                        _ => break,
                    }
                }
            });
            tokio::select! {
                _ = worker_cancel.cancelled() => {},
                _ = child.wait() => {},
                _ = &mut reader => {},
                _ = &mut writer_task => {},
            }
            worker_cancel.cancel();
            // start_kill targets the Child handle, never a guessed process ID.
            let _ = child.start_kill();
            let _ = child.wait().await;
            reader.abort();
            writer_task.abort();
        });
        let mut bridge = Self {
            writer, pending, slots: Arc::new(Semaphore::new(MAX_PENDING)),
            next_id: AtomicU64::new(1), events: Some(events), cancel,
            owner: Some(owner), command_timeout: config.command_timeout, ready: Value::Null,
        };
        bridge.ready = match timeout(config.startup_timeout, ready_rx).await {
            Ok(Ok(ready)) => ready,
            Ok(Err(_)) => {
                bridge.shutdown().await;
                return Err(VoiceRpcError::Closed);
            }
            Err(_) => {
                bridge.shutdown().await;
                return Err(VoiceRpcError::Timeout);
            }
        };
        Ok(bridge)
    }

    pub fn ready(&self) -> &Value { &self.ready }

    /// Take the sole event receiver before starting voice or registering tools.
    /// Values are original OMP frames, including `host_tool_cancel`.
    pub fn take_events(&mut self) -> Option<mpsc::Receiver<Value>> { self.events.take() }

    pub async fn set_host_tools(&self, tools: Vec<Value>) -> Result<Value, VoiceRpcError> {
        self.command("set_host_tools", json!({"tools": tools})).await
    }

    pub async fn live_connect(&self, sdp_offer: &str, voice: Option<&str>, instructions: Option<&str>) -> Result<Value, VoiceRpcError> {
        let mut fields = json!({"sdpOffer": sdp_offer});
        if let Some(voice) = voice { fields["voice"] = json!(voice); }
        if let Some(instructions) = instructions { fields["instructions"] = json!(instructions); }
        self.command("live_connect", fields).await
    }

    pub async fn live_stop(&self) -> Result<Value, VoiceRpcError> {
        self.command("live_stop", json!({})).await
    }

    /// Synchronize OMP's mute phase; the caller must also mute its audio track.
    pub async fn live_mute(&self, muted: bool) -> Result<Value, VoiceRpcError> {
        self.command("live_mute", json!({"muted": muted})).await
    }

    /// Complete a `host_tool_call` using its RPC `id`, not its `toolCallId`.
    /// `result` is the OMP host-tool result object; `is_error` rejects the call.
    /// A successful return means queued for writing, not acknowledged by OMP.
    pub async fn host_tool_result(&self, id: &str, result: Value, is_error: bool) -> Result<(), VoiceRpcError> {
        self.send(json!({"type": "host_tool_result", "id": id, "result": result, "isError": is_error})).await
    }

    /// Kill and reap the owned child. This is safe even if live_stop timed out.
    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(owner) = self.owner.take() { let _ = owner.await; }
    }

    async fn command(&self, command: &'static str, mut fields: Value) -> Result<Value, VoiceRpcError> {
        let deadline = tokio::time::Instant::now() + self.command_timeout;
        let permit = tokio::select! {
            _ = self.cancel.cancelled() => return Err(VoiceRpcError::Closed),
            result = tokio::time::timeout_at(deadline, self.slots.clone().acquire_owned()) =>
                result.map_err(|_| VoiceRpcError::Timeout)?.map_err(|_| VoiceRpcError::Closed)?,
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        fields["id"] = json!(id);
        fields["type"] = json!(command);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().map_err(|_| VoiceRpcError::Closed)?.insert(id.clone(), tx);
        let _guard = PendingGuard { id, pending: self.pending.clone() };
        let _permit = permit;
        tokio::select! {
            _ = self.cancel.cancelled() => return Err(VoiceRpcError::Closed),
            result = tokio::time::timeout_at(deadline, self.send(fields)) => result.map_err(|_| VoiceRpcError::Timeout)??,
        }
        let response = tokio::select! {
            _ = self.cancel.cancelled() => return Err(VoiceRpcError::Closed),
            result = tokio::time::timeout_at(deadline, rx) => result.map_err(|_| VoiceRpcError::Timeout)?.map_err(|_| VoiceRpcError::Closed)?,
        };
        if response.get("command").and_then(Value::as_str) != Some(command) { return Err(VoiceRpcError::Protocol); }
        match response.get("success").and_then(Value::as_bool) {
            Some(true) => Ok(response.get("data").cloned().unwrap_or(Value::Null)),
            Some(false) => Err(VoiceRpcError::Remote(response.get("error").and_then(Value::as_str).unwrap_or("unknown RPC error").to_owned())),
            None => Err(VoiceRpcError::Protocol),
        }
    }

    async fn send(&self, frame: Value) -> Result<(), VoiceRpcError> {
        let mut bytes = LimitedFrame(Vec::new());
        serde_json::to_writer(&mut bytes, &frame).map_err(|_| VoiceRpcError::Protocol)?;
        let mut bytes = bytes.0;
        bytes.push(b'\n');
        tokio::select! {
            _ = self.cancel.cancelled() => Err(VoiceRpcError::Closed),
            result = timeout(self.command_timeout, self.writer.send(bytes)) =>
                result.map_err(|_| VoiceRpcError::Timeout)?.map_err(|_| VoiceRpcError::Closed),
        }
    }
}

// Reserve one byte for the newline without buffering an oversized JSON value.
struct LimitedFrame(Vec<u8>);
impl std::io::Write for LimitedFrame {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > (MAX_FRAME - 1).saturating_sub(self.0.len()) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "RPC frame too large"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl Drop for VoiceRpc {
    fn drop(&mut self) { self.cancel.cancel(); }
}

async fn read_frames(
    stdout: tokio::process::ChildStdout,
    ready_tx: oneshot::Sender<Value>,
    events: mpsc::Sender<Value>,
    pending: Pending,
    cancel: CancellationToken,
) -> Result<(), VoiceRpcError> {
    let mut reader = BufReader::new(stdout);
    let first = read_frame(&mut reader).await?;
    if first.get("type").and_then(Value::as_str) != Some("ready")
        || first.get("protocolVersion").and_then(Value::as_u64) != Some(1)
    { return Err(VoiceRpcError::Protocol); }
    let _ = ready_tx.send(first);
    loop {
        let frame = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            frame = read_frame(&mut reader) => frame?,
        };
        match frame.get("type").and_then(Value::as_str) {
            Some("response") => {
                if let Some(id) = frame.get("id").and_then(Value::as_str) {
                    let sender = pending.lock().map_err(|_| VoiceRpcError::Closed)?.remove(id);
                    if let Some(sender) = sender { let _ = sender.send(frame); }
                }
            },
            Some("live_phase" | "live_transcript" | "live_end" | "host_tool_call" | "host_tool_cancel") => {
                events.try_send(frame).map_err(|_| VoiceRpcError::Closed)?;
            },
            Some("rpc_frame_error" | "rpc_chunk") | None => return Err(VoiceRpcError::Protocol),
            _ => {},
        }
    }
}

// `read_until` alone can allocate without limit when the peer omits a newline.
async fn read_frame(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Value, VoiceRpcError> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().await.map_err(|_| VoiceRpcError::Closed)?;
        if buffer.is_empty() { return Err(VoiceRpcError::Closed); }
        let end = buffer.iter().position(|b| *b == b'\n');
        let consumed = end.map_or(buffer.len(), |i| i + 1);
        if bytes.len() + consumed > MAX_FRAME { return Err(VoiceRpcError::Protocol); }
        bytes.extend_from_slice(&buffer[..consumed]);
        reader.consume(consumed);
        if end.is_some() { break; }
    }
    let frame: Value = serde_json::from_slice(&bytes).map_err(|_| VoiceRpcError::Protocol)?;
    if !frame.is_object() { return Err(VoiceRpcError::Protocol); }
    Ok(frame)
}
