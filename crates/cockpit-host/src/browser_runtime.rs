use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::browser_annotations::AnnotationServer;
use cockpit_core::{InspectionError, browser::BrowserService};
use cockpit_protocol::browser::{
    BrowserFeedbackAckRequest, BrowserFeedbackImage, BrowserFeedbackImageRequest,
    BrowserFeedbackLookup, BrowserFeedbackRequest, BrowserFeedbackSendRequest,
    BrowserFeedbackSendResponse, BrowserRequest, BrowserResponse,
};
use cockpit_protocol::browser_feedback::BrowserFeedbackAck;
use fs2::FileExt;
use nix::unistd::Uid;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore, oneshot},
    time::timeout,
};

const MAX_REQUEST_FRAME: usize = 256 * 1024;
const MAX_RESPONSE_FRAME: usize = 129 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const OWNER_READY_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_READY_POLL: Duration = Duration::from_millis(25);
const OWNER_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_PEERS: usize = 32;
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum WireRequest {
    Action(BrowserRequest),
    Feedback(BrowserFeedbackRequest),
    Acknowledge(BrowserFeedbackAckRequest),
    FeedbackImage(BrowserFeedbackImageRequest),
    SendFeedback(BrowserFeedbackSendRequest),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum WireResponse {
    Action(BrowserResponse),
    Feedback(BrowserFeedbackLookup),
    Acknowledged(BrowserFeedbackAck),
    FeedbackImage(BrowserFeedbackImage),
    FeedbackSent(BrowserFeedbackSendResponse),
    Err(WireError),
}

#[derive(Debug, Serialize, Deserialize)]
struct WireError {
    code: String,
    message: String,
}

struct Owner {
    lock: File,
    stop: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
    annotations: Option<AnnotationServer>,
}

enum RuntimeRole {
    Owner(Owner),
    Observer { socket: PathBuf },
}

/// Browser host transport. Exactly one runtime owns a state root; other local
/// clients forward requests to the owner over its private Unix socket.
pub struct BrowserRuntime {
    role: Mutex<RuntimeRole>,
    service: Arc<BrowserService>,
}

impl BrowserRuntime {
    pub async fn connect(
        state_root: PathBuf,
        service: Arc<BrowserService>,
    ) -> Result<Self, InspectionError> {
        let state_root = state_root.join("browser");
        let lock_path = state_root.join("owner.lock");
        let socket = state_root.join("owner.sock");
        let deadline = Instant::now() + OWNER_READY_TIMEOUT;
        let lock = open_owner_lock(&lock_path, deadline).await?;
        verify_lock(&lock)?;
        let expected_uid = lock
            .metadata()
            .map_err(|error| io_error("browser_owner_unavailable", error))?
            .uid();
        wait_for_owner(&socket, &lock, expected_uid, deadline, false).await?;
        Ok(Self {
            role: Mutex::new(RuntimeRole::Observer { socket }),
            service,
        })
    }

    pub async fn start(
        state_root: PathBuf,
        service: Arc<BrowserService>,
    ) -> Result<Self, InspectionError> {
        let state_root = state_root.join("browser");
        fs::create_dir_all(&state_root)
            .map_err(|error| io_error("browser_state_unavailable", error))?;
        fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700))
            .map_err(|error| io_error("browser_state_unavailable", error))?;
        let lock_path = state_root.join("owner.lock");
        let socket = state_root.join("owner.sock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&lock_path)
            .map_err(|error| io_error("browser_owner_unavailable", error))?;
        verify_lock(&lock)?;
        match lock.try_lock_exclusive() {
            Ok(()) => {
                match fs::symlink_metadata(&socket) {
                    Ok(metadata)
                        if metadata.file_type().is_socket()
                            && metadata.uid() == Uid::current().as_raw() =>
                    {
                        fs::remove_file(&socket)
                            .map_err(|error| io_error("browser_owner_unavailable", error))?;
                    }
                    Ok(_) => {
                        return Err(InspectionError::new(
                            "unsafe_path",
                            "Browser owner socket path is not an owned socket",
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(io_error("browser_owner_unavailable", error)),
                }
                let listener = UnixListener::bind(&socket)
                    .map_err(|error| io_error("browser_owner_unavailable", error))?;
                fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
                    .map_err(|error| io_error("browser_owner_unavailable", error))?;
                let expected_uid = lock
                    .metadata()
                    .map_err(|error| io_error("browser_owner_unavailable", error))?
                    .uid();
                let annotations = AnnotationServer::start(Arc::clone(&service)).await?;
                let peers = Arc::new(Semaphore::new(MAX_PEERS));
                let (stop_tx, mut stop_rx) = oneshot::channel();
                let owner_service = Arc::clone(&service);
                let cleanup_socket = socket.clone();
                let mut reconcile = tokio::time::interval(Duration::from_secs(15));
                let task = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = &mut stop_rx => break,
                            _ = reconcile.tick() => {
                                let _ = owner_service.reconcile().await;
                                let _ = owner_service.prune_feedback();
                            }
                            accepted = listener.accept() => {
                                let Ok((stream, _)) = accepted else { continue };
                                let Ok(peer_uid) = stream.peer_cred().map(|cred| cred.uid()) else { continue };
                                if peer_uid != expected_uid { continue; }
                                let Ok(permit) = Arc::clone(&peers).try_acquire_owned() else { continue; };
                                let service = Arc::clone(&owner_service);
                                tokio::spawn(async move { let _permit = permit; serve_peer(stream, service).await; });
                            }
                        }
                    }
                    let _ = fs::remove_file(cleanup_socket);
                });
                Ok(Self {
                    role: Mutex::new(RuntimeRole::Owner(Owner {
                        lock,
                        stop: Some(stop_tx),
                        task: Some(task),
                        annotations: Some(annotations),
                    })),
                    service,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let expected_uid = lock
                    .metadata()
                    .map_err(|error| io_error("browser_owner_unavailable", error))?
                    .uid();
                let deadline = Instant::now() + OWNER_READY_TIMEOUT;
                wait_for_owner(&socket, &lock, expected_uid, deadline, true).await?;
                Ok(Self {
                    role: Mutex::new(RuntimeRole::Observer { socket }),
                    service,
                })
            }
            Err(error) => Err(io_error("browser_owner_unavailable", error)),
        }
    }

    pub async fn execute(
        &self,
        request: BrowserRequest,
    ) -> Result<BrowserResponse, InspectionError> {
        match self.request(WireRequest::Action(request)).await? {
            WireResponse::Action(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn feedback(
        &self,
        request: BrowserFeedbackRequest,
    ) -> Result<BrowserFeedbackLookup, InspectionError> {
        match self.request(WireRequest::Feedback(request)).await? {
            WireResponse::Feedback(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn feedback_image(
        &self,
        request: BrowserFeedbackImageRequest,
    ) -> Result<BrowserFeedbackImage, InspectionError> {
        match self.request(WireRequest::FeedbackImage(request)).await? {
            WireResponse::FeedbackImage(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn send_feedback(
        &self,
        request: BrowserFeedbackSendRequest,
    ) -> Result<BrowserFeedbackSendResponse, InspectionError> {
        match self.request(WireRequest::SendFeedback(request)).await? {
            WireResponse::FeedbackSent(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }
    pub async fn acknowledge_feedback(
        &self,
        request: BrowserFeedbackAckRequest,
    ) -> Result<BrowserFeedbackAck, InspectionError> {
        match self.request(WireRequest::Acknowledge(request)).await? {
            WireResponse::Acknowledged(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    async fn request(&self, request: WireRequest) -> Result<WireResponse, InspectionError> {
        let socket = {
            let role = self.role.lock().await;
            match &*role {
                RuntimeRole::Owner(_) => None,
                RuntimeRole::Observer { socket } => Some(socket.clone()),
            }
        };
        match socket {
            None => dispatch(&self.service, request).await,
            Some(socket) => forward(&socket, request).await,
        }
    }

    /// Owner shutdown closes only its associations. Observer shutdown leaves the owner intact.
    pub async fn shutdown(&self) -> Result<(), InspectionError> {
        let owner = {
            let mut role = self.role.lock().await;
            match &mut *role {
                RuntimeRole::Owner(owner) => (
                    owner.stop.take(),
                    owner.task.take(),
                    owner.annotations.take(),
                ),
                RuntimeRole::Observer { .. } => (None, None, None),
            }
        };
        if owner.0.is_some() {
            let service_result = self.service.shutdown().await;
            if let Some(mut annotations) = owner.2 {
                annotations.shutdown().await;
            }
            if let Some(stop) = owner.0 {
                let _ = stop.send(());
            }
            if let Some(task) = owner.1 {
                let _ = task.await;
            }
            let mut role = self.role.lock().await;
            if let RuntimeRole::Owner(owner) = &mut *role {
                let _ = owner.lock.unlock();
            }
            return service_result;
        }
        Ok(())
    }

    pub async fn is_owner(&self) -> bool {
        matches!(*self.role.lock().await, RuntimeRole::Owner(_))
    }
}

async fn dispatch(
    service: &BrowserService,
    request: WireRequest,
) -> Result<WireResponse, InspectionError> {
    match request {
        WireRequest::Action(request) => service.execute(request).await.map(WireResponse::Action),
        WireRequest::Feedback(request) => service
            .feedback(&request.target)
            .await
            .map(WireResponse::Feedback),
        WireRequest::Acknowledge(request) => service
            .acknowledge_feedback(request)
            .await
            .map(WireResponse::Acknowledged),
        WireRequest::FeedbackImage(request) => service
            .feedback_image(request)
            .await
            .map(WireResponse::FeedbackImage),
        WireRequest::SendFeedback(request) => service
            .send_feedback(request)
            .await
            .map(WireResponse::FeedbackSent),
    }
}

fn invalid_response() -> InspectionError {
    InspectionError::new(
        "invalid_browser_response",
        "Browser owner returned an unexpected response kind",
    )
}

async fn serve_peer(stream: UnixStream, service: Arc<BrowserService>) {
    let (mut reader, mut writer) = stream.into_split();
    let Ok(Ok(Some(frame))) = timeout(IO_TIMEOUT, read_frame(&mut reader, MAX_REQUEST_FRAME)).await
    else {
        return;
    };
    let response = match serde_json::from_slice::<WireRequest>(&frame) {
        Ok(request) => match dispatch(&service, request).await {
            Ok(value) => value,
            Err(error) => WireResponse::Err(WireError {
                code: error.code,
                message: error.message,
            }),
        },
        Err(error) => WireResponse::Err(WireError {
            code: "invalid_browser_request".to_owned(),
            message: error.to_string(),
        }),
    };
    let Ok(encoded) = serde_json::to_vec(&response) else {
        return;
    };
    if encoded.len() > MAX_RESPONSE_FRAME {
        return;
    }
    let _ = timeout(IO_TIMEOUT, async {
        writer.write_all(&encoded).await?;
        writer.write_all(b"\n").await
    })
    .await;
}

async fn forward(socket: &Path, request: WireRequest) -> Result<WireResponse, InspectionError> {
    let stream = timeout(IO_TIMEOUT, UnixStream::connect(socket))
        .await
        .map_err(|_| {
            InspectionError::new(
                "browser_owner_timeout",
                "Browser owner connection timed out",
            )
        })?
        .map_err(|error| io_error("browser_owner_unavailable", error))?;
    let peer_uid = stream
        .peer_cred()
        .map_err(|error| io_error("browser_owner_unavailable", error))?
        .uid();
    let expected_uid = fs::metadata(socket)
        .map_err(|error| io_error("browser_owner_unavailable", error))?
        .uid();
    if peer_uid != expected_uid {
        return Err(InspectionError::new(
            "browser_owner_unavailable",
            "Browser owner belongs to another user",
        ));
    }
    let (mut reader, mut writer) = stream.into_split();
    let encoded = serde_json::to_vec(&request)
        .map_err(|error| InspectionError::new("invalid_browser_request", error.to_string()))?;
    if encoded.len() > MAX_REQUEST_FRAME {
        return Err(InspectionError::new(
            "browser_request_too_large",
            "Browser request exceeds the bounded frame limit",
        ));
    }
    timeout(IO_TIMEOUT, async {
        writer.write_all(&encoded).await?;
        writer.write_all(b"\n").await
    })
    .await
    .map_err(|_| {
        InspectionError::new(
            "browser_outcome_unknown",
            "Browser request write timed out; inspect status before another mutation",
        )
    })?
    .map_err(|error| io_error("browser_outcome_unknown", error))?;
    let frame = timeout(IO_TIMEOUT, read_frame(&mut reader, MAX_RESPONSE_FRAME))
        .await
        .map_err(|_| {
            InspectionError::new(
                "browser_outcome_unknown",
                "Browser response timed out; inspect status before another mutation",
            )
        })?
        .map_err(|error| io_error("browser_outcome_unknown", error))?
        .ok_or_else(|| {
            InspectionError::new(
                "browser_outcome_unknown",
                "Browser owner closed the request; inspect status before another mutation",
            )
        })?;
    match serde_json::from_slice::<WireResponse>(&frame)
        .map_err(|error| InspectionError::new("invalid_browser_response", error.to_string()))?
    {
        WireResponse::Err(error) => Err(InspectionError::new(error.code, error.message)),
        response => Ok(response),
    }
}

async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            return Ok((!frame.is_empty()).then_some(frame));
        }
        if let Some(end) = chunk[..count].iter().position(|byte| *byte == b'\n') {
            if frame.len() + end > limit {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "frame exceeds bound",
                ));
            }
            frame.extend_from_slice(&chunk[..end]);
            return Ok(Some(frame));
        }
        if frame.len() + count > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame exceeds bound",
            ));
        }
        frame.extend_from_slice(&chunk[..count]);
    }
}

fn verify_lock(lock: &File) -> Result<(), InspectionError> {
    let metadata = lock
        .metadata()
        .map_err(|error| io_error("browser_owner_unavailable", error))?;
    if !metadata.is_file()
        || metadata.uid() != Uid::current().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(InspectionError::new(
            "unsafe_path",
            "Browser owner lock is not a private owned file",
        ));
    }
    Ok(())
}

enum DescriptorReadiness {
    Ready,
    Retry,
    Reject(InspectionError),
}

fn probe_descriptor(socket: &Path, lock: &File) -> DescriptorReadiness {
    let metadata = match fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return DescriptorReadiness::Retry;
        }
        Err(error) => {
            return DescriptorReadiness::Reject(io_error("browser_owner_unavailable", error));
        }
    };
    if !metadata.file_type().is_socket() {
        return DescriptorReadiness::Reject(InspectionError::new(
            "browser_owner_unavailable",
            "Browser owner socket is not private",
        ));
    }
    let lock_uid = match lock.metadata() {
        Ok(metadata) => metadata.uid(),
        Err(error) => {
            return DescriptorReadiness::Reject(io_error("browser_owner_unavailable", error));
        }
    };
    if metadata.uid() != lock_uid || lock_uid != Uid::current().as_raw() {
        return DescriptorReadiness::Reject(InspectionError::new(
            "browser_owner_unavailable",
            "Browser owner belongs to another user",
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        return DescriptorReadiness::Retry;
    }
    DescriptorReadiness::Ready
}

fn owner_lock_held(lock: &File) -> Result<bool, InspectionError> {
    match lock.try_lock_exclusive() {
        Ok(()) => {
            lock.unlock()
                .map_err(|error| io_error("browser_owner_unavailable", error))?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(io_error("browser_owner_unavailable", error)),
    }
}

enum PeerReadiness {
    Ready,
    Retry,
    Reject(InspectionError),
}

async fn probe_live_peer(
    socket: &Path,
    expected_uid: u32,
    timeout_duration: Duration,
) -> PeerReadiness {
    let stream = match timeout(timeout_duration, UnixStream::connect(socket)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) if retryable_peer_error(&error) => return PeerReadiness::Retry,
        Ok(Err(error)) => {
            return PeerReadiness::Reject(io_error("browser_owner_unavailable", error));
        }
        Err(_) => return PeerReadiness::Retry,
    };
    let uid = match stream.peer_cred() {
        Ok(cred) => cred.uid(),
        Err(error) => {
            return PeerReadiness::Reject(io_error("browser_owner_unavailable", error));
        }
    };
    if uid != expected_uid {
        return PeerReadiness::Reject(InspectionError::new(
            "browser_owner_unavailable",
            "Browser owner belongs to another user",
        ));
    }
    PeerReadiness::Ready
}

fn retryable_peer_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
    )
}

async fn open_owner_lock(path: &Path, deadline: Instant) -> Result<File, InspectionError> {
    loop {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)
        {
            Ok(lock) => return Ok(lock),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("browser_owner_unavailable", error)),
        }
        if Instant::now() >= deadline {
            return Err(owner_ready_timeout());
        }
        tokio::time::sleep(OWNER_READY_POLL).await;
    }
}

async fn wait_for_owner(
    socket: &Path,
    lock: &File,
    expected_uid: u32,
    deadline: Instant,
    require_lock_before_probe: bool,
) -> Result<(), InspectionError> {
    loop {
        if Instant::now() >= deadline {
            return Err(owner_ready_timeout());
        }
        if require_lock_before_probe && !owner_lock_held(lock)? {
            return Err(owner_exited_before_ready());
        }
        match probe_descriptor(socket, lock) {
            DescriptorReadiness::Ready => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(owner_ready_timeout());
                }
                match probe_live_peer(socket, expected_uid, OWNER_PROBE_TIMEOUT.min(remaining))
                    .await
                {
                    PeerReadiness::Ready => {
                        if owner_lock_held(lock)? {
                            return Ok(());
                        }
                        return Err(owner_exited_before_ready());
                    }
                    PeerReadiness::Retry => {}
                    PeerReadiness::Reject(error) => return Err(error),
                }
            }
            DescriptorReadiness::Retry => {}
            DescriptorReadiness::Reject(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(owner_ready_timeout());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(OWNER_READY_POLL.min(remaining)).await;
    }
}

fn owner_exited_before_ready() -> InspectionError {
    InspectionError::new(
        "browser_owner_unavailable",
        "Browser owner exited before its private socket became ready; start Cockpit or `cockpit serve` with the same configuration",
    )
}

fn owner_ready_timeout() -> InspectionError {
    InspectionError::new(
        "browser_owner_timeout",
        "Browser owner did not become ready before the bounded startup wait elapsed; start Cockpit or `cockpit serve` with the same configuration",
    )
}

fn io_error(code: &'static str, error: impl std::fmt::Display) -> InspectionError {
    let message = if code == "browser_owner_unavailable" {
        format!(
            "Browser owner unavailable ({error}). Start Cockpit or `cockpit serve` with the same configuration"
        )
    } else {
        error.to_string()
    };
    InspectionError::new(code, message)
}
