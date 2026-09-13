use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[path = "browser_helper.rs"]
mod browser_helper;

use browser_helper::BrowserHelperSupervisor;
pub use browser_helper::{BrowserViewEvents, BrowserViewOpen};
use cockpit_core::{InspectionError, browser::BrowserService};
use cockpit_protocol::browser::{
    BrowserFeedbackAckRequest, BrowserFeedbackImage, BrowserFeedbackImageRequest,
    BrowserFeedbackLookup, BrowserFeedbackRequest, BrowserFeedbackSendRequest,
    BrowserFeedbackSendResponse, BrowserRequest, BrowserResponse,
};
use cockpit_protocol::browser_feedback::BrowserFeedbackAck;
use cockpit_protocol::browser_view::{
    BrowserViewCommand, BrowserViewCommandOutcome, BrowserViewCommandRequest, BrowserViewCommandResponse, BrowserViewDraftCommand, BrowserViewFrameGrant,
    BrowserViewOpenRequest, BrowserDraftRecoveryRequest,
};

use fs2::FileExt;
use nix::unistd::Uid;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore, oneshot},
    time::timeout,
};

const MAX_REQUEST_FRAME: usize = 6 * 1024 * 1024;
const MAX_RESPONSE_FRAME: usize = 129 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
// Owner browser actions can perform up to four sequential 15-second CLI
// probes/operations, with a small allowance for the bounded socket checks
// around them. This remains a finite observer deadline.
const BROWSER_ACTION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(75);
// Attachment performs one 15-second live probe before the helper's bounded
// 10-second ready and 10-second first-frame waits.
const INLINE_VIEW_OPEN_RESPONSE_TIMEOUT: Duration = Duration::from_secs(45);
// A helper command waits up to 10 seconds for its response; leave bounded
// room for the owner's follow-up work before returning the wire response.
const INLINE_VIEW_COMMAND_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const OWNER_READY_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_READY_POLL: Duration = Duration::from_millis(25);
const OWNER_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_PEERS: usize = 32;
const VIEW_EVENT_QUEUE: usize = 64;

fn response_read_timeout(request: &WireRequest) -> Duration {
    match request {
        WireRequest::Action(_) => BROWSER_ACTION_RESPONSE_TIMEOUT,
        WireRequest::BrowserViewOpen(_) => INLINE_VIEW_OPEN_RESPONSE_TIMEOUT,
        WireRequest::BrowserViewCommand(_) => INLINE_VIEW_COMMAND_RESPONSE_TIMEOUT,
        _ => IO_TIMEOUT,
    }
}

/// Validate a browser-view command and preserve the protocol's structured
/// navigation rejection code at every host boundary.
pub fn validate_browser_view_command(
    request: &BrowserViewCommandRequest,
) -> Result<(), InspectionError> {
    request.validate().map_err(|message| {
        if let Some(message) = message
            .strip_prefix("invalid_browser_url:")
            .map(str::trim_start)
        {
            InspectionError::new("invalid_browser_url", message)
        } else {
            InspectionError::new("invalid_browser_view_command", message)
        }
    })
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum WireRequest {
    Action(BrowserRequest),
    Feedback(BrowserFeedbackRequest),
    Acknowledge(BrowserFeedbackAckRequest),
    FeedbackImage(BrowserFeedbackImageRequest),
    SendFeedback(BrowserFeedbackSendRequest),
    BrowserViewOpen(BrowserViewOpenRequest),
    BrowserViewCommand(BrowserViewCommandRequest),
    BrowserViewEvents(String),
    BrowserViewFrameEndpoint(BrowserViewFrameGrant),
    BrowserViewDetach(String),
    BrowserDraftRecovery(BrowserDraftRecoveryRequest),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum WireResponse {
    Action(BrowserResponse),
    Feedback(BrowserFeedbackLookup),
    Acknowledged(BrowserFeedbackAck),
    FeedbackImage(BrowserFeedbackImage),
    FeedbackSent(BrowserFeedbackSendResponse),
    BrowserViewOpen(WireBrowserViewOpen),
    BrowserViewCommand(BrowserViewCommandResponse),
    BrowserViewEvent(cockpit_protocol::browser_view::BrowserViewEvent),
    BrowserViewFrameEndpoint(String),
    BrowserViewDetached,
    BrowserDraftRecovery(BrowserViewCommandOutcome),
    Err(WireError),
}

#[derive(Debug, Serialize, Deserialize)]
struct WireBrowserViewOpen {
    snapshot: cockpit_protocol::browser_view::BrowserViewSnapshot,
    first_frame: cockpit_protocol::browser_view::BrowserViewFrameDescriptor,
    frame_endpoint: String,
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
    /// Present only in the local owner. Observers intentionally cannot attach
    /// to, command, or subscribe to the owner's private browser helper.
    helper: Option<Arc<BrowserHelperSupervisor>>,
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
            helper: None,
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
                let peers = Arc::new(Semaphore::new(MAX_PEERS));
                let (stop_tx, mut stop_rx) = oneshot::channel();
                let owner_service = Arc::clone(&service);
                let cleanup_socket = socket.clone();
                let mut reconcile = tokio::time::interval(Duration::from_secs(15));
                let helper = Arc::new(BrowserHelperSupervisor::new(state_root.clone()));
                let task_helper = Arc::clone(&helper);
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
                                let helper = Arc::clone(&task_helper);
                                tokio::spawn(async move { let _permit = permit; serve_peer(stream, service, helper).await; });
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
                    })),
                    service,
                    helper: Some(helper),
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
                    helper: None,
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

    /// Attach a persistent inline view to a freshly verified local association.
    /// The returned endpoint is loopback-only; authenticate its WebSocket with
    /// the snapshot's first-message frame grant, not a query parameter.
    pub async fn open_browser_view(
        &self,
        request: BrowserViewOpenRequest,
    ) -> Result<BrowserViewOpen, InspectionError> {
        if let Some(helper) = self.helper.as_ref() {
            let attachment = self.service.browser_runtime_attachment(&request).await?;
            return helper.open(attachment, request).await;
        }
        match self.request(WireRequest::BrowserViewOpen(request)).await? {
            WireResponse::BrowserViewOpen(open) => Ok(BrowserViewOpen {
                snapshot: open.snapshot,
                first_frame: open.first_frame,
                frame_endpoint: open.frame_endpoint,
            }),
            _ => Err(invalid_response()),
        }
    }

    pub async fn browser_draft_recovery(
        &self, request: BrowserDraftRecoveryRequest,
    ) -> Result<BrowserViewCommandOutcome, InspectionError> {
        match self.request(WireRequest::BrowserDraftRecovery(request)).await? {
            WireResponse::BrowserDraftRecovery(outcome) => Ok(outcome),
            _ => Err(invalid_response()),
        }
    }

    pub async fn browser_view_events(
        &self,
        view_id: &str,
    ) -> Result<BrowserViewEvents, InspectionError> {
        if let Some(helper) = self.helper.as_ref() {
            return helper.events(view_id).await;
        }
        let socket = self.owner_socket().await?;
        forward_view_events(&socket, view_id).await
    }

    pub async fn browser_view_command(
        &self,
        request: BrowserViewCommandRequest,
    ) -> Result<BrowserViewCommandResponse, InspectionError> {
        validate_browser_view_command(&request)?;
        if let Some(helper) = self.helper.as_ref() {
            return owner_view_command(&self.service, helper, request).await;
        }
        match self
            .request(WireRequest::BrowserViewCommand(request))
            .await?
        {
            WireResponse::BrowserViewCommand(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn browser_view_frame_endpoint(
        &self,
        grant: &BrowserViewFrameGrant,
    ) -> Result<String, InspectionError> {
        if let Some(helper) = self.helper.as_ref() {
            return helper.frame_endpoint(grant).await;
        }
        match self
            .request(WireRequest::BrowserViewFrameEndpoint(grant.clone()))
            .await?
        {
            WireResponse::BrowserViewFrameEndpoint(endpoint) => Ok(endpoint),
            _ => Err(invalid_response()),
        }
    }

    pub async fn browser_view_detach(&self, view_id: &str) -> Result<(), InspectionError> {
        if let Some(helper) = self.helper.as_ref() {
            helper.detach(view_id).await;
            return Ok(());
        }
        match self
            .request(WireRequest::BrowserViewDetach(view_id.to_owned()))
            .await?
        {
            WireResponse::BrowserViewDetached => Ok(()),
            _ => Err(invalid_response()),
        }
    }

    async fn owner_socket(&self) -> Result<PathBuf, InspectionError> {
        let role = self.role.lock().await;
        match &*role {
            RuntimeRole::Observer { socket } => Ok(socket.clone()),
            RuntimeRole::Owner(_) => Err(InspectionError::new(
                "browser_owner_unavailable",
                "Browser owner socket is unavailable",
            )),
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
            None => dispatch(&self.service, self.helper.as_ref(), request).await,
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
                ),
                RuntimeRole::Observer { .. } => (None, None),
            }
        };
        if owner.0.is_some() {
            if let Some(helper) = &self.helper {
                helper.shutdown().await;
            }
            let service_result = self.service.shutdown().await;
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
fn invalid_response() -> InspectionError {
    InspectionError::new(
        "invalid_browser_response",
        "Browser owner returned an unexpected response kind",
    )
}

async fn dispatch(
    service: &BrowserService,
    helper: Option<&Arc<BrowserHelperSupervisor>>,
    request: WireRequest,
) -> Result<WireResponse, InspectionError> {
    match request {
        WireRequest::BrowserDraftRecovery(request) => service.browser_draft_recovery(request).await.map(WireResponse::BrowserDraftRecovery),
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
        WireRequest::BrowserViewOpen(request) => {
            let helper = helper.ok_or_else(|| {
                InspectionError::new(
                    "browser_view_owner_required",
                    "Inline browser owner helper is unavailable",
                )
            })?;
            let attachment = service.browser_runtime_attachment(&request).await?;
            let opened = helper.open(attachment, request).await?;
            Ok(WireResponse::BrowserViewOpen(WireBrowserViewOpen {
                snapshot: opened.snapshot,
                first_frame: opened.first_frame,
                frame_endpoint: opened.frame_endpoint,
            }))
        }
        WireRequest::BrowserViewCommand(request) => {
            validate_browser_view_command(&request)?;
            let helper = helper.ok_or_else(|| {
                InspectionError::new(
                    "browser_view_owner_required",
                    "Inline browser owner helper is unavailable",
                )
            })?;
            owner_view_command(service, helper, request)
                .await
                .map(WireResponse::BrowserViewCommand)
        }
        WireRequest::BrowserViewFrameEndpoint(grant) => {
            let helper = helper.ok_or_else(|| {
                InspectionError::new(
                    "browser_view_owner_required",
                    "Inline browser owner helper is unavailable",
                )
            })?;
            helper
                .frame_endpoint(&grant)
                .await
                .map(WireResponse::BrowserViewFrameEndpoint)
        }
        WireRequest::BrowserViewDetach(view_id) => {
            if let Some(helper) = helper {
                helper.detach(&view_id).await;
            }
            Ok(WireResponse::BrowserViewDetached)
        }
        WireRequest::BrowserViewEvents(_) => Err(InspectionError::new(
            "invalid_browser_request",
            "Browser view events require a streaming connection",
        )),
    }
}

async fn owner_view_command(
    service: &BrowserService,
    helper: &BrowserHelperSupervisor,
    request: BrowserViewCommandRequest,
) -> Result<BrowserViewCommandResponse, InspectionError> {
    if let BrowserViewCommand::Draft { context, draft_id, expected_revision, command } = &request.command {
        let view = helper.events(&request.view_id).await?;
        let snapshot = view.snapshot;
        let historical = matches!(command, BrowserViewDraftCommand::List | BrowserViewDraftCommand::SaveCapture { .. }
            | BrowserViewDraftCommand::RetryPending | BrowserViewDraftCommand::DiscardPending);
        if snapshot.identity.stream_epoch != request.stream_epoch
            || (!historical && snapshot.document.as_ref().is_none_or(|document|
                document.target_id != context.target_id || document.document_generation != context.document_generation))
        {
            return Ok(BrowserViewCommandResponse::Stale {
                view_id: request.view_id,
                stream_epoch: request.stream_epoch,
                request_id: request.request_id,
                current_stream_epoch: snapshot.identity.stream_epoch,
                current_metadata_sequence: snapshot.metadata_sequence,
                code: "browser_document_stale".into(),
                message: "Browser document changed; reopen the draft from the current view".into(),
            });
        }
        let (target, attachment) = helper.attachment_context(&request.view_id).await?;
        let result = service.browser_annotation_command(
            &target, &attachment, context.clone(), draft_id.as_deref(), *expected_revision, command.clone(),
        ).await;
        return Ok(match result {
            Ok(outcome) => BrowserViewCommandResponse::Accepted {
                view_id: request.view_id, stream_epoch: request.stream_epoch,
                request_id: request.request_id, outcome,
            },
            Err(error) => BrowserViewCommandResponse::Rejected {
                view_id: request.view_id, stream_epoch: request.stream_epoch,
                request_id: request.request_id, code: error.code, message: error.message,
            },
        });
    }
    let response = helper.command(request.clone()).await?;
    if let (BrowserViewCommand::Capture { command }, BrowserViewCommandResponse::Accepted {
        outcome: BrowserViewCommandOutcome::CapturePrepared { capture_id, descriptor }, ..
    }) = (&request.command, &response) {
        let (target, attachment) = helper.attachment_context(&request.view_id).await?;
        let snapshot = helper.events(&request.view_id).await?.snapshot;
        let document = snapshot.document.filter(|document|
            document.target_id == descriptor.target_id && document.document_generation == descriptor.document_generation
        ).ok_or_else(|| InspectionError::new("browser_capture_stale", "The document changed during capture preparation"))?;
        service.browser_prepare_capture(&target, &attachment, command, capture_id, descriptor,
            document.frame_id, document.frame_generation).await?;
    }
    Ok(response)
}

async fn serve_peer(
    stream: UnixStream,
    service: Arc<BrowserService>,
    helper: Arc<BrowserHelperSupervisor>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let Ok(Ok(Some(frame))) = timeout(IO_TIMEOUT, read_frame(&mut reader, MAX_REQUEST_FRAME)).await
    else {
        return;
    };
    let request = match serde_json::from_slice::<WireRequest>(&frame) {
        Ok(request) => request,
        Err(error) => {
            let _ = write_wire(
                &mut writer,
                &WireResponse::Err(WireError {
                    code: "invalid_browser_request".into(),
                    message: error.to_string(),
                }),
            )
            .await;
            return;
        }
    };
    if let WireRequest::BrowserViewEvents(view_id) = request {
        let Ok(view) = helper.events(&view_id).await else {
            return;
        };
        let metadata = cockpit_protocol::browser_view::BrowserViewEventMetadata {
            view_id: view.snapshot.identity.view_id.clone(),
            stream_epoch: view.snapshot.identity.stream_epoch,
            metadata_sequence: view.snapshot.metadata_sequence,
        };
        let attached = cockpit_protocol::browser_view::BrowserViewEvent::Attached {
            metadata,
            snapshot: view.snapshot,
        };
        if write_wire(&mut writer, &WireResponse::BrowserViewEvent(attached))
            .await
            .is_err()
        {
            return;
        }
        let mut events = view.events;
        let mut peer_byte = [0u8; 1];
        loop {
            tokio::select! {
                _ = reader.read(&mut peer_byte) => break,
                event = events.recv() => {
                    let Ok(event) = event else { break; };
                    if write_wire(&mut writer, &WireResponse::BrowserViewEvent(event)).await.is_err() {
                        break;
                    }
                }
            }
        }
        return;
    }
    let response = match dispatch(&service, Some(&helper), request).await {
        Ok(value) => value,
        Err(error) => WireResponse::Err(WireError {
            code: error.code,
            message: error.message,
        }),
    };
    let _ = write_wire(&mut writer, &response).await;
}

async fn write_wire<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: &WireResponse,
) -> Result<(), std::io::Error> {
    let encoded = serde_json::to_vec(response).map_err(std::io::Error::other)?;
    if encoded.len() > MAX_RESPONSE_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "response exceeds bound",
        ));
    }
    timeout(IO_TIMEOUT, async {
        writer.write_all(&encoded).await?;
        writer.write_all(b"\n").await
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "response timed out"))?
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
    let response_timeout = response_read_timeout(&request);
    let frame = timeout(response_timeout, read_frame(&mut reader, MAX_RESPONSE_FRAME))
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

async fn forward_view_events(
    socket: &Path,
    view_id: &str,
) -> Result<BrowserViewEvents, InspectionError> {
    let stream = UnixStream::connect(socket)
        .await
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
    let request = serde_json::to_vec(&WireRequest::BrowserViewEvents(view_id.to_owned()))
        .map_err(|error| InspectionError::new("invalid_browser_request", error.to_string()))?;
    writer
        .write_all(&request)
        .await
        .map_err(|error| io_error("browser_owner_unavailable", error))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|error| io_error("browser_owner_unavailable", error))?;
    let first = timeout(IO_TIMEOUT, read_frame(&mut reader, MAX_RESPONSE_FRAME))
        .await
        .map_err(|_| {
            InspectionError::new("browser_owner_timeout", "Browser event snapshot timed out")
        })?
        .map_err(|error| io_error("browser_owner_unavailable", error))?
        .ok_or_else(|| {
            InspectionError::new(
                "browser_view_not_found",
                "Browser owner closed the event stream",
            )
        })?;
    let response = serde_json::from_slice::<WireResponse>(&first)
        .map_err(|error| InspectionError::new("invalid_browser_response", error.to_string()))?;
    let WireResponse::BrowserViewEvent(
        cockpit_protocol::browser_view::BrowserViewEvent::Attached { snapshot, .. },
    ) = response
    else {
        return Err(InspectionError::new(
            "invalid_browser_response",
            "Browser owner did not return a browser view snapshot",
        ));
    };
    let (events, receiver) = tokio::sync::broadcast::channel(VIEW_EVENT_QUEUE);
    let relay = events.clone();
    tokio::spawn(async move {
        // Keep the client-to-owner half open while the returned receiver is
        // live; the owner uses EOF on this half as the event-stream signal.
        let _writer = writer;
        loop {
            let frame = tokio::select! {
                _ = relay.closed() => break,
                frame = read_frame(&mut reader, MAX_RESPONSE_FRAME) => frame,
            };
            let Ok(Some(frame)) = frame else {
                break;
            };
            let Ok(WireResponse::BrowserViewEvent(event)) = serde_json::from_slice(&frame) else {
                break;
            };
            if relay.send(event).is_err() {
                break;
            }
        }
    });
    Ok(BrowserViewEvents {
        snapshot,
        events: receiver,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_read_deadline_matches_browser_operation() {
        let action: WireRequest = serde_json::from_str(
            r#"{"kind":"action","value":{"target":{"session_id":"session","space_id":null,"pane_id":null,"endpoint_path":null},"action":{"kind":"status"}}}"#,
        )
        .expect("valid action request");
        let open: WireRequest = serde_json::from_str(
            r#"{"kind":"browser_view_open","value":{"target":{"session_id":"session","space_id":null,"pane_id":null,"endpoint_path":null},"client_id":"client","presentation":"split","viewport":{"css_width":800,"css_height":600,"device_pixel_ratio":1.0},"takeover":false}}"#,
        )
        .expect("valid browser view open request");
        let command: WireRequest = serde_json::from_str(
            r#"{"kind":"browser_view_command","value":{"view_id":"view","stream_epoch":1,"request_id":"request","command":{"type":"take_control","viewport":{"css_width":800,"css_height":600,"device_pixel_ratio":1.0}}}}"#,
        )
        .expect("valid browser view command request");

        assert_eq!(
            response_read_timeout(&action),
            BROWSER_ACTION_RESPONSE_TIMEOUT
        );
        assert_eq!(
            response_read_timeout(&open),
            INLINE_VIEW_OPEN_RESPONSE_TIMEOUT
        );
        assert_eq!(
            response_read_timeout(&command),
            INLINE_VIEW_COMMAND_RESPONSE_TIMEOUT
        );
        assert_eq!(
            response_read_timeout(&WireRequest::BrowserViewEvents("view".to_owned())),
            IO_TIMEOUT
        );
    }
}
