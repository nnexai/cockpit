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
pub use browser_helper::{BrowserViewEvents, BrowserViewNativeSubscription, BrowserViewOpen};
use cockpit_core::{InspectionError, browser::BrowserService, widget::{WidgetService, WidgetWindowGuard}};
use cockpit_protocol::browser::{
    BrowserCleanupRetryRequest, BrowserCleanupStatus, BrowserFeedbackAckRequest,
    BrowserFeedbackImage, BrowserFeedbackImageRequest, BrowserFeedbackLookup,
    BrowserFeedbackRequest, BrowserFeedbackSendRequest, BrowserFeedbackSendResponse,
    BrowserRequest, BrowserResponse,
};
use cockpit_protocol::browser_feedback::BrowserFeedbackAck;
use cockpit_protocol::browser_view::{
    BrowserViewCommand, BrowserViewCommandOutcome, BrowserViewCommandRequest, BrowserViewCommandResponse, BrowserViewDraftCommand, BrowserViewFrameGrant,
    BrowserViewOpenRequest, BrowserDraftRecoveryRequest,
};
use cockpit_protocol::widget::*;

use fs2::FileExt;
use nix::unistd::Uid;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
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
        WireRequest::Action(_) | WireRequest::CleanupRetry(_) => BROWSER_ACTION_RESPONSE_TIMEOUT,
        WireRequest::BrowserViewOpen(_) => INLINE_VIEW_OPEN_RESPONSE_TIMEOUT,
        WireRequest::BrowserViewCommand(_) => INLINE_VIEW_COMMAND_RESPONSE_TIMEOUT,
        WireRequest::WidgetShow(_) | WireRequest::WidgetClose(_) | WireRequest::WidgetList(_) =>
            Duration::from_secs(45),
        WireRequest::WidgetSelection(request) =>
            Duration::from_secs(request.wait_seconds.unwrap_or(0).min(WIDGET_MAX_WAIT_SECONDS) + 45),
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
    CleanupStatus,
    CleanupRetry(BrowserCleanupRetryRequest),
    BrowserViewOpen(BrowserViewOpenRequest),
    BrowserViewCommand(BrowserViewCommandRequest),
    BrowserViewEvents(String),
    BrowserViewFrameEndpoint(BrowserViewFrameGrant),
    BrowserViewDetach(String),
    BrowserDraftRecovery(BrowserDraftRecoveryRequest),
    WidgetShow(WidgetShowRequest),
    WidgetClose(WidgetCloseRequest),
    WidgetList(WidgetListRequest),
    WidgetSelection(WidgetSelectionRequest),
    WidgetContent(WidgetContentRequest),
    WidgetRemove(WidgetRemoveRequest),
    WidgetSelect(WidgetSelectRequest),
    WidgetWindowReport { window_id: String, report: WidgetWindowReport },
    WidgetEvents,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum WireResponse {
    Action(BrowserResponse),
    Feedback(BrowserFeedbackLookup),
    Acknowledged(BrowserFeedbackAck),
    FeedbackImage(BrowserFeedbackImage),
    FeedbackSent(BrowserFeedbackSendResponse),
    CleanupStatus(BrowserCleanupStatus),
    BrowserViewOpen(WireBrowserViewOpen),
    BrowserViewCommand(BrowserViewCommandResponse),
    BrowserViewEvent(cockpit_protocol::browser_view::BrowserViewEvent),
    BrowserViewFrameEndpoint(String),
    BrowserViewDetached,
    BrowserDraftRecovery(BrowserViewCommandOutcome),
    WidgetShown(WidgetShowResponse),
    WidgetClosed(WidgetCloseResponse),
    WidgetListed(WidgetListResponse),
    WidgetSelection(WidgetSelectionResponse),
    WidgetContent(WidgetContent),
    WidgetRemoved(WidgetRemoveResponse),
    WidgetSelected(WidgetSelectResponse),
    WidgetReported,
    WidgetSubscribed { window_id: String },
    WidgetEvent(WidgetEvent),
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
    socket: PathBuf,
    socket_device: u64,
    socket_inode: u64,
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
    widgets: Option<Arc<WidgetService>>,
    /// Present only in the local owner. Observers intentionally cannot attach
    /// to, command, or subscribe to the owner's private browser helper.
    helper: Option<Arc<BrowserHelperSupervisor>>,
}

/// Keep the complete stream alive: dropping it unregisters this window.
pub struct WidgetEventStream {
    pub window_id: String,
    pub snapshot: WidgetEvent,
    pub events: tokio::sync::broadcast::Receiver<WidgetEvent>,
    _guard: WidgetStreamGuard,
}

enum WidgetStreamGuard {
    Owner { _window: WidgetWindowGuard },
    Observer(tokio::task::JoinHandle<()>),
}

impl Drop for WidgetStreamGuard {
    fn drop(&mut self) {
        if let Self::Observer(task) = self {
            task.abort();
        }
    }
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
            widgets: None,
        })
    }

    pub async fn start(
        state_root: PathBuf,
        service: Arc<BrowserService>,
        widgets: Arc<WidgetService>,
    ) -> Result<Self, InspectionError> {
        let owner_state_root = state_root;
        let state_root = owner_state_root.join("browser");
        ensure_private_state_root(&state_root)?;
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
                cockpit_core::ephemeral::reset_owner_state(&owner_state_root, &service).await?;
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
                let socket_metadata = fs::symlink_metadata(&socket)
                    .map_err(|error| io_error("browser_owner_unavailable", error))?;
                let socket_device = socket_metadata.dev();
                let socket_inode = socket_metadata.ino();
                let expected_uid = lock
                    .metadata()
                    .map_err(|error| io_error("browser_owner_unavailable", error))?
                    .uid();
                let peers = Arc::new(Semaphore::new(MAX_PEERS));
                let (stop_tx, mut stop_rx) = oneshot::channel();
                let (widget_stop, _) = tokio::sync::watch::channel(false);
                let owner_service = Arc::clone(&service);
                let owner_widgets = Arc::clone(&widgets);
                let cleanup_socket = socket.clone();
                let cleanup_device = socket_device;
                let cleanup_inode = socket_inode;
                let mut reconcile = tokio::time::interval_at(
                    tokio::time::Instant::now() + Duration::from_secs(15),
                    Duration::from_secs(15),
                );
                let mut endpoint_probe = tokio::time::interval(Duration::from_millis(250));
                endpoint_probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let helper = Arc::new(BrowserHelperSupervisor::new(state_root.clone()));
                let mut retired = helper.take_retired_receiver().await.expect("retirement receiver available");
                let task_helper = Arc::clone(&helper);
                let task = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = &mut stop_rx => break,
                            _ = reconcile.tick() => {
                                let _ = owner_service.reconcile().await;
                                let _ = owner_service.prune_feedback();
                                owner_widgets.reconcile().await;
                            }
                            _ = endpoint_probe.tick() => {
                                task_helper.verify_active_endpoints(&owner_service).await;
                            }
                            Some(retired_view) = retired.recv() => {
                                task_helper.retire_view(retired_view).await;
                            }
                            accepted = listener.accept() => {
                                let Ok((stream, _)) = accepted else { continue };
                                let Ok(peer_uid) = stream.peer_cred().map(|cred| cred.uid()) else { continue };
                                if peer_uid != expected_uid { continue; }
                                let Ok(permit) = Arc::clone(&peers).try_acquire_owned() else { continue; };
                                let service = Arc::clone(&owner_service);
                                let helper = Arc::clone(&task_helper);
                                let widgets = Arc::clone(&owner_widgets);
                                let stopped = widget_stop.subscribe();
                                tokio::spawn(async move { let _permit = permit; serve_peer(stream, service, helper, widgets, stopped).await; });
                            }
                        }
                    }
                    owner_widgets.shutdown();
                    widget_stop.send_replace(true);
                    if let Ok(metadata) = fs::symlink_metadata(&cleanup_socket)
                        && metadata.file_type().is_socket()
                        && metadata.dev() == cleanup_device
                        && metadata.ino() == cleanup_inode
                    {
                        let _ = fs::remove_file(cleanup_socket);
                    }
                });
                Ok(Self {
                    role: Mutex::new(RuntimeRole::Owner(Owner {
                        lock,
                        socket: socket.clone(),
                        socket_device,
                        socket_inode,
                        stop: Some(stop_tx),
                        task: Some(task),
                    })),
                    service,
                    widgets: Some(widgets),
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
                    widgets: None,
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

    pub async fn cleanup_status(&self) -> Result<BrowserCleanupStatus, InspectionError> {
        match self.request(WireRequest::CleanupStatus).await? {
            WireResponse::CleanupStatus(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn retry_cleanup(
        &self,
        request: BrowserCleanupRetryRequest,
    ) -> Result<BrowserCleanupStatus, InspectionError> {
        match self.request(WireRequest::CleanupRetry(request)).await? {
            WireResponse::CleanupStatus(response) => Ok(response),
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
            self.service.verify_browser_runtime_endpoint(&attachment).await?;
            let opened = helper.open(attachment.clone(), request).await?;
            if let Err(error) = self.service.verify_browser_runtime_endpoint(&attachment).await {
                helper.revoke_attachment(&attachment).await;
                return Err(error);
            }
            return Ok(opened);
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
            verify_view_endpoint(&self.service, helper, view_id).await?;
            return helper.events(view_id).await;
        }
        let socket = self.owner_socket().await?;
        forward_view_events(&socket, view_id).await
    }
    pub async fn browser_view_native_subscribe(
        &self,
        view_id: &str,
        stream_epoch: u64,
    ) -> Result<BrowserViewNativeSubscription, InspectionError> {
        if let Some(helper) = self.helper.as_ref() {
            verify_view_endpoint(&self.service, helper, view_id).await?;
            return helper.native_subscribe(view_id, stream_epoch).await;
        }
        let events = self.browser_view_events(view_id).await?;
        if events.snapshot.identity.stream_epoch != stream_epoch {
            return Err(InspectionError::new(
                "stale_browser_view",
                "Browser view stream epoch is stale",
            ));
        }
        let grant = events.snapshot.frame_grant.clone().ok_or_else(|| {
            InspectionError::new(
                "browser_frame_unavailable",
                "Browser view has no frame grant",
            )
        })?;
        let endpoint = self.browser_view_frame_endpoint(&grant).await?;
        Ok(BrowserViewNativeSubscription {
            snapshot: events.snapshot,
            events: events.events,
            endpoint,
            grant,
        })
    }

    pub async fn browser_view_native_release(&self, view_id: &str) {
        if let Some(helper) = self.helper.as_ref() {
            if let Ok((_, attachment)) = helper.attachment_context(view_id).await
                && self
                    .service
                    .verify_browser_runtime_endpoint(&attachment)
                    .await
                    .is_err()
            {
                helper.revoke_attachment(&attachment).await;
                return;
            }
            helper.native_release(view_id).await;
        } else {
            let _ = self.browser_view_detach(view_id).await;
        }
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
            verify_view_endpoint(&self.service, helper, &grant.view_id).await?;
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
            verify_view_endpoint(&self.service, helper, view_id).await?;
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

    pub async fn widget_show(&self, request: WidgetShowRequest) -> Result<WidgetShowResponse, InspectionError> {
        match self.request(WireRequest::WidgetShow(request)).await? {
            WireResponse::WidgetShown(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_close(&self, request: WidgetCloseRequest) -> Result<WidgetCloseResponse, InspectionError> {
        match self.request(WireRequest::WidgetClose(request)).await? {
            WireResponse::WidgetClosed(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_list(&self, request: WidgetListRequest) -> Result<WidgetListResponse, InspectionError> {
        match self.request(WireRequest::WidgetList(request)).await? {
            WireResponse::WidgetListed(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_selection(&self, request: WidgetSelectionRequest) -> Result<WidgetSelectionResponse, InspectionError> {
        let response = self.request(WireRequest::WidgetSelection(request)).await.map_err(|error| {
            if error.code == "browser_outcome_unknown" {
                InspectionError::new("widget_retired", "Widget owner ended the selection request")
            } else {
                error
            }
        })?;
        match response {
            WireResponse::WidgetSelection(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_content(&self, request: WidgetContentRequest) -> Result<WidgetContent, InspectionError> {
        match self.request(WireRequest::WidgetContent(request)).await? {
            WireResponse::WidgetContent(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_remove(&self, request: WidgetRemoveRequest) -> Result<WidgetRemoveResponse, InspectionError> {
        match self.request(WireRequest::WidgetRemove(request)).await? {
            WireResponse::WidgetRemoved(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_select(&self, request: WidgetSelectRequest) -> Result<WidgetSelectResponse, InspectionError> {
        match self.request(WireRequest::WidgetSelect(request)).await? {
            WireResponse::WidgetSelected(response) => Ok(response),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_report(&self, window_id: &str, report: WidgetWindowReport) -> Result<(), InspectionError> {
        match self.request(WireRequest::WidgetWindowReport { window_id: window_id.to_owned(), report }).await? {
            WireResponse::WidgetReported => Ok(()),
            _ => Err(invalid_response()),
        }
    }

    pub async fn widget_events(&self) -> Result<WidgetEventStream, InspectionError> {
        let socket = {
            let role = self.role.lock().await;
            match &*role {
                RuntimeRole::Owner(_) => None,
                RuntimeRole::Observer { socket } => Some(socket.clone()),
            }
        };
        if let Some(socket) = socket {
            return forward_widget_events(&socket).await;
        }
        let subscription = widget_service(self.widgets.as_deref())?.subscribe();
        Ok(WidgetEventStream {
            window_id: subscription.window_id,
            snapshot: subscription.snapshot,
            events: subscription.events,
            _guard: WidgetStreamGuard::Owner { _window: subscription.guard },
        })
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
            None => dispatch(&self.service, self.helper.as_ref(), self.widgets.as_deref(), request).await,
            Some(socket) => forward(&socket, request).await,
        }
    }

    /// Owner shutdown closes only its associations. Observer shutdown leaves the owner intact.
    pub async fn shutdown(&self) -> Result<(), InspectionError> {
        let owner = {
            let mut role = self.role.lock().await;
            match &mut *role {
                RuntimeRole::Owner(owner)
                    if owner.stop.is_some() || owner.task.is_some() =>
                {
                    Some((
                        owner.stop.take(),
                        owner.task.take(),
                        owner.socket.clone(),
                        owner.socket_device,
                        owner.socket_inode,
                    ))
                }
                RuntimeRole::Owner(_) | RuntimeRole::Observer { .. } => None,
            }
        };
        let Some((stop, mut task, socket, socket_device, socket_inode)) = owner else {
            return Ok(());
        };
        if let Some(widgets) = &self.widgets {
            widgets.shutdown();
        }
        if let Some(helper) = &self.helper {
            helper.shutdown().await;
        }
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        if let Some(mut task) = task.take() {
            if timeout(Duration::from_secs(2), &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        if let Ok(metadata) = fs::symlink_metadata(&socket)
            && metadata.file_type().is_socket()
            && metadata.dev() == socket_device
            && metadata.ino() == socket_inode
        {
            let _ = fs::remove_file(&socket);
        }
        let service_result = self.service.shutdown().await;
        let mut role = self.role.lock().await;
        if let RuntimeRole::Owner(owner) = &mut *role {
            let _ = owner.lock.unlock();
        }
        service_result
    }

    pub async fn is_owner(&self) -> bool {
        matches!(*self.role.lock().await, RuntimeRole::Owner(_))
    }
}
fn ensure_private_state_root(path: &Path) -> Result<(), InspectionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(InspectionError::new(
                "unsafe_path",
                "Browser state root must be an owned regular directory",
            ))
        }
        Ok(_) => verify_private_state_owner(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| io_error("browser_state_unavailable", error))?;
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    Err(InspectionError::new(
                        "unsafe_path",
                        "Browser state root changed to a non-directory",
                    ))
                }
                Ok(_) => verify_private_state_owner(path),
                Err(error) => Err(io_error("browser_state_unavailable", error)),
            }
        }
        Err(error) => Err(io_error("browser_state_unavailable", error)),
    }
}

fn verify_private_state_owner(path: &Path) -> Result<(), InspectionError> {
    #[cfg(unix)]
    {
        let metadata = fs::metadata(path)
            .map_err(|error| io_error("browser_state_unavailable", error))?;
        if metadata.uid() != Uid::current().as_raw() {
            return Err(InspectionError::new(
                "unsafe_path",
                "Browser state belongs to another user",
            ));
        }
    }
    Ok(())
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
    widgets: Option<&WidgetService>,
    request: WireRequest,
) -> Result<WireResponse, InspectionError> {
    match request {
        WireRequest::WidgetShow(request) => widget_service(widgets)?.show(request).await.map(WireResponse::WidgetShown),
        WireRequest::WidgetClose(request) => widget_service(widgets)?.close(request).await.map(WireResponse::WidgetClosed),
        WireRequest::WidgetList(request) => widget_service(widgets)?.list(request).await.map(WireResponse::WidgetListed),
        WireRequest::WidgetSelection(request) => widget_service(widgets)?.selection(request).await.map(WireResponse::WidgetSelection),
        WireRequest::WidgetContent(request) => widget_service(widgets)?.content(request).map(WireResponse::WidgetContent),
        WireRequest::WidgetRemove(request) => widget_service(widgets)?.remove(request).map(WireResponse::WidgetRemoved),
        WireRequest::WidgetSelect(request) => widget_service(widgets)?.select(request).map(WireResponse::WidgetSelected),
        WireRequest::WidgetWindowReport { window_id, report } => {
            widget_service(widgets)?.report_window(&window_id, report)?;
            Ok(WireResponse::WidgetReported)
        }
        WireRequest::WidgetEvents => Err(InspectionError::new("widget_usage", "Widget events require a streaming connection")),
        WireRequest::BrowserDraftRecovery(request) => service.browser_draft_recovery(request).await.map(WireResponse::BrowserDraftRecovery),
        WireRequest::Action(request) => service.execute(request).await.map(WireResponse::Action),
        WireRequest::CleanupStatus => service.cleanup_status().await.map(WireResponse::CleanupStatus),
        WireRequest::CleanupRetry(request) => service
            .retry_cleanup(request)
            .await
            .map(WireResponse::CleanupStatus),
        WireRequest::Feedback(request) => service
            .feedback(&request.scope)
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
            service.verify_browser_runtime_endpoint(&attachment).await?;
            let opened = helper.open(attachment.clone(), request).await?;
            if let Err(error) = service.verify_browser_runtime_endpoint(&attachment).await {
                helper.revoke_attachment(&attachment).await;
                return Err(error);
            }
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
            verify_view_endpoint(service, helper, &grant.view_id).await?;
            helper
                .frame_endpoint(&grant)
                .await
                .map(WireResponse::BrowserViewFrameEndpoint)
        }
        WireRequest::BrowserViewDetach(view_id) => {
            if let Some(helper) = helper {
                verify_view_endpoint(service, helper, &view_id).await?;
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

fn widget_service(widgets: Option<&WidgetService>) -> Result<&WidgetService, InspectionError> {
    widgets.ok_or_else(|| InspectionError::new("widget_no_owner", "Widget owner is unavailable"))
}

async fn verify_view_endpoint(
    service: &BrowserService,
    helper: &BrowserHelperSupervisor,
    view_id: &str,
) -> Result<(), InspectionError> {
    let (_, attachment) = helper.attachment_context(view_id).await?;
    if let Err(error) = service.verify_browser_runtime_endpoint(&attachment).await {
        helper.revoke_attachment(&attachment).await;
        return Err(error);
    }
    Ok(())
}

async fn owner_view_command(
    service: &BrowserService,
    helper: &BrowserHelperSupervisor,
    request: BrowserViewCommandRequest,
) -> Result<BrowserViewCommandResponse, InspectionError> {
    verify_view_endpoint(service, helper, &request.view_id).await?;
    if matches!(&request.command, &BrowserViewCommand::Detach) {
        let snapshot = helper.events(&request.view_id).await?.snapshot;
        if snapshot.identity.stream_epoch != request.stream_epoch {
            return Ok(BrowserViewCommandResponse::Stale {
                view_id: request.view_id,
                stream_epoch: request.stream_epoch,
                request_id: request.request_id,
                current_stream_epoch: snapshot.identity.stream_epoch,
                current_metadata_sequence: snapshot.metadata_sequence,
                code: "stale_stream".into(),
                message: "Browser view stream identity changed".into(),
            });
        }
        helper.detach(&request.view_id).await;
        return Ok(BrowserViewCommandResponse::Accepted {
            view_id: request.view_id,
            stream_epoch: request.stream_epoch,
            request_id: request.request_id,
            outcome: BrowserViewCommandOutcome::None,
        });
    }
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
        verify_view_endpoint(service, helper, &request.view_id).await?;
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
        verify_view_endpoint(service, helper, &request.view_id).await?;
        service.browser_prepare_capture(&target, &attachment, command, capture_id, descriptor,
            document.frame_id, document.frame_generation).await?;
    }
    Ok(response)
}

async fn serve_peer(
    stream: UnixStream,
    service: Arc<BrowserService>,
    helper: Arc<BrowserHelperSupervisor>,
    widgets: Arc<WidgetService>,
    mut owner_stopped: tokio::sync::watch::Receiver<bool>,
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
    if matches!(request, WireRequest::WidgetEvents) {
        let subscription = widgets.subscribe();
        let _guard = subscription.guard;
        if write_wire(&mut writer, &WireResponse::WidgetSubscribed { window_id: subscription.window_id }).await.is_err()
            || write_wire(&mut writer, &WireResponse::WidgetEvent(subscription.snapshot)).await.is_err()
        {
            return;
        }
        let mut events = subscription.events;
        let mut peer_byte = [0u8; 1];
        loop {
            tokio::select! {
                _ = owner_stopped.changed() => break,
                _ = reader.read(&mut peer_byte) => break,
                event = events.recv() => {
                    let Ok(event) = event else { break; };
                    if write_wire(&mut writer, &WireResponse::WidgetEvent(event)).await.is_err() {
                        break;
                    }
                }
            }
        }
        return;
    }
    if let WireRequest::BrowserViewEvents(view_id) = request {
        if verify_view_endpoint(&service, &helper, &view_id).await.is_err() {
            return;
        }
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
    // Selection waits are read-only. Drop their dispatch future on disconnect
    // so its WaitGuard releases the globally bounded waiter slot immediately.
    // Ordinary mutations must finish even when their client disappears.
    let waiting_selection = matches!(
        &request,
        WireRequest::WidgetSelection(request) if request.wait_seconds.is_some_and(|seconds| seconds > 0)
    );
    let dispatched = if waiting_selection {
        if *owner_stopped.borrow() {
            return;
        }
        tokio::select! {
            biased;
            _ = owner_stopped.changed() => return,
            _ = async {
                let mut chunk = [0u8; 4096];
                loop {
                    match reader.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            } => return,
            response = dispatch(&service, Some(&helper), Some(&widgets), request) => response,
        }
    } else {
        dispatch(&service, Some(&helper), Some(&widgets), request).await
    };
    let response = match dispatched {
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

async fn forward_widget_events(socket: &Path) -> Result<WidgetEventStream, InspectionError> {
    let stream = timeout(IO_TIMEOUT, UnixStream::connect(socket)).await
        .map_err(|_| InspectionError::new("browser_owner_timeout", "Widget owner connection timed out"))?
        .map_err(|error| io_error("browser_owner_unavailable", error))?;
    let peer_uid = stream.peer_cred()
        .map_err(|error| io_error("browser_owner_unavailable", error))?.uid();
    let expected_uid = fs::metadata(socket)
        .map_err(|error| io_error("browser_owner_unavailable", error))?.uid();
    if peer_uid != expected_uid {
        return Err(InspectionError::new("browser_owner_unavailable", "Widget owner belongs to another user"));
    }
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let request = serde_json::to_vec(&WireRequest::WidgetEvents)
        .map_err(|error| InspectionError::new("widget_usage", error.to_string()))?;
    timeout(IO_TIMEOUT, async {
        writer.write_all(&request).await?;
        writer.write_all(b"\n").await
    }).await
        .map_err(|_| InspectionError::new("browser_owner_timeout", "Widget subscription timed out"))?
        .map_err(|error| io_error("browser_owner_unavailable", error))?;
    let subscribed = read_widget_stream_frame(&mut reader).await?;
    let WireResponse::WidgetSubscribed { window_id } = subscribed else {
        return Err(invalid_response());
    };
    let snapshot = read_widget_stream_frame(&mut reader).await?;
    let WireResponse::WidgetEvent(snapshot @ WidgetEvent::Snapshot { .. }) = snapshot else {
        return Err(invalid_response());
    };
    let (events, receiver) = tokio::sync::broadcast::channel(VIEW_EVENT_QUEUE);
    let task = tokio::spawn(async move {
        let _writer = writer;
        loop {
            let frame = tokio::select! {
                _ = events.closed() => break,
                frame = read_stream_frame(&mut reader, MAX_RESPONSE_FRAME) => frame,
            };
            let Ok(Some(frame)) = frame else { break; };
            let Ok(WireResponse::WidgetEvent(event)) = serde_json::from_slice(&frame) else { break; };
            if events.send(event).is_err() { break; }
        }
    });
    Ok(WidgetEventStream { window_id, snapshot, events: receiver, _guard: WidgetStreamGuard::Observer(task) })
}

async fn read_widget_stream_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<WireResponse, InspectionError> {
    let frame = timeout(IO_TIMEOUT, read_stream_frame(reader, MAX_RESPONSE_FRAME)).await
        .map_err(|_| InspectionError::new("browser_owner_timeout", "Widget event snapshot timed out"))?
        .map_err(|error| io_error("browser_owner_unavailable", error))?
        .ok_or_else(|| InspectionError::new("widget_retired", "Widget owner closed the event stream"))?;
    match serde_json::from_slice(&frame)
        .map_err(|error| InspectionError::new("invalid_browser_response", error.to_string()))?
    {
        WireResponse::Err(error) => Err(InspectionError::new(error.code, error.message)),
        response => Ok(response),
    }
}

// A stream reader must retain bytes following the first newline: subscription,
// snapshot and subsequent events can all arrive in the same socket read.
async fn read_stream_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::with_capacity(limit.min(4096));
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok((!frame.is_empty()).then_some(frame));
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let count = newline.unwrap_or(chunk.len());
        if count > limit - frame.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame exceeds bound",
            ));
        }
        frame.extend_from_slice(&chunk[..count]);
        reader.consume(count + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(frame));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cockpit_core::browser::{BrowserHerdrAdapter, BrowserHerdrSnapshot};
    use cockpit_core::config::BrowserConfiguration;
    use std::{future::{Future, poll_fn}, task::Poll};
    use sha2::{Digest, Sha256};

    struct OfflineHerdr;

    #[async_trait]
    impl BrowserHerdrAdapter for OfflineHerdr {
        async fn browser_snapshot(
            &self,
            _session_id: &str,
        ) -> Result<BrowserHerdrSnapshot, InspectionError> {
            Err(InspectionError::new("herdr_unavailable", "Herdr is offline"))
        }
    }

    #[async_trait]
    impl cockpit_core::paste_adapter::CommentPasteAdapter for OfflineHerdr {
        async fn comment_paste_targets(&self, _: &str) -> Result<Vec<cockpit_protocol::comment_paste::CommentPasteTarget>, InspectionError> {
            Err(InspectionError::new("herdr_unavailable", "Herdr is offline"))
        }
        async fn focus_comment_paste_target(&self, _: &cockpit_protocol::comment_paste::CommentPasteTarget) -> Result<(), InspectionError> {
            Err(InspectionError::new("herdr_unavailable", "Herdr is offline"))
        }
        async fn confirm_comment_paste_target_focus(&self, _: &cockpit_protocol::comment_paste::CommentPasteTarget) -> Result<(), InspectionError> {
            Err(InspectionError::new("herdr_unavailable", "Herdr is offline"))
        }
        async fn send_comment_paste(&self, _: &cockpit_protocol::comment_paste::CommentPasteTarget, _: &str) -> Result<(), InspectionError> {
            Err(InspectionError::new("herdr_unavailable", "Herdr is offline"))
        }
    }

    struct WidgetHerdr;

    #[async_trait]
    impl BrowserHerdrAdapter for WidgetHerdr {
        async fn browser_snapshot(&self, _: &str) -> Result<BrowserHerdrSnapshot, InspectionError> {
            Ok(BrowserHerdrSnapshot {
                endpoint_identity: "widget-fixture".into(),
                endpoint_path: "/fixture/herdr.sock".into(),
                snapshot: serde_json::from_value(serde_json::json!({
                    "session_id": "daily", "server_instance": "fixture", "version": "fixture", "protocol": 22,
                    "focused_space_id": "s1", "focused_tab_id": "t1", "focused_pane_id": "p1",
                    "spaces": [{"id":"s1","label":"Test","number":1,"tab_count":1,"pane_count":1,
                        "focused":true,"agent_status":"idle","git":null}],
                    "tabs": [{"id":"t1","space_id":"s1","label":"Test","number":1,"pane_count":1,
                        "focused":true,"focused_pane_id":"p1"}],
                    "panes": [{"id":"p1","terminal_id":"term1","space_id":"s1","tab_id":"t1",
                        "focused":true,"agent_status":"idle","revision":0}],
                    "agents": []
                })).unwrap(),
            })
        }
    }

    #[async_trait]
    impl cockpit_core::paste_adapter::CommentPasteAdapter for WidgetHerdr {
        async fn comment_paste_targets(&self, _: &str) -> Result<Vec<cockpit_protocol::comment_paste::CommentPasteTarget>, InspectionError> {
            Ok(Vec::new())
        }
        async fn focus_comment_paste_target(&self, _: &cockpit_protocol::comment_paste::CommentPasteTarget) -> Result<(), InspectionError> {
            panic!("widget operations must not focus a terminal")
        }
        async fn confirm_comment_paste_target_focus(&self, _: &cockpit_protocol::comment_paste::CommentPasteTarget) -> Result<(), InspectionError> {
            panic!("widget operations must not request paste focus")
        }
        async fn send_comment_paste(&self, _: &cockpit_protocol::comment_paste::CommentPasteTarget, _: &str) -> Result<(), InspectionError> {
            panic!("widget operations must not paste into a terminal")
        }
    }

    struct RuntimeFixture(PathBuf);

    impl RuntimeFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("cbrt-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn service(&self) -> Arc<BrowserService> {
            Arc::new(
                BrowserService::new(
                    BrowserConfiguration {
                        playwright_cli: self.0.join("unavailable-playwright-cli"),
                        default_url: "about:blank".into(),
                        chromium_executable: None,
                        node_executable: None,
                        browser_helper: None,
                        playwright_core: None,
                        feedback_retention_seconds: 3600,
                        feedback_max_store_bytes: 1024 * 1024,
                    },
                    self.0.clone(),
                    Arc::new(OfflineHerdr),
                )
                .unwrap(),
            )
        }

        fn widgets(&self) -> Arc<WidgetService> {
            Arc::new(WidgetService::new(Arc::new(OfflineHerdr), Arc::new(OfflineHerdr)))
        }
    }

    impl Drop for RuntimeFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn owner_start_resets_work_but_observers_preserve_it_until_replacement() {
        let fixture = RuntimeFixture::new();
        let service = fixture.service();
        let work_dirs = ["browser", "comments", "review"];
        for dir in work_dirs {
            fs::create_dir_all(fixture.0.join(dir)).unwrap();
            fs::write(fixture.0.join(dir).join("previous-run"), b"old work").unwrap();
        }
        let project = fixture.0.join("project.json");
        fs::write(&project, b"project state").unwrap();
        let owner = BrowserRuntime::start(fixture.0.clone(), service, fixture.widgets()).await.unwrap();
        let lock_inode = fs::metadata(fixture.0.join("browser/owner.lock")).unwrap().ino();
        assert!(owner.is_owner().await);
        for dir in work_dirs {
            assert!(!fixture.0.join(dir).join("previous-run").exists());
            fs::write(fixture.0.join(dir).join("current-run"), b"live work").unwrap();
        }

        let observer = BrowserRuntime::connect(fixture.0.clone(), fixture.service()).await.unwrap();
        assert!(!observer.is_owner().await);
        for dir in work_dirs {
            assert_eq!(fs::read(fixture.0.join(dir).join("current-run")).unwrap(), b"live work");
        }
        observer.shutdown().await.unwrap();
        // Closing an observer must not relinquish ownership or discard in-run work.
        let next_window = BrowserRuntime::start(fixture.0.clone(), fixture.service(), fixture.widgets()).await.unwrap();
        assert!(!next_window.is_owner().await);
        assert!(next_window.cleanup_status().await.unwrap().failures.is_empty());
        for dir in work_dirs {
            assert_eq!(fs::read(fixture.0.join(dir).join("current-run")).unwrap(), b"live work");
        }
        next_window.shutdown().await.unwrap();

        owner.shutdown().await.unwrap();
        let replacement = BrowserRuntime::start(fixture.0.clone(), fixture.service(), fixture.widgets()).await.unwrap();
        assert!(replacement.is_owner().await);
        for dir in work_dirs {
            assert!(!fixture.0.join(dir).join("current-run").exists());
        }
        assert_eq!(fs::metadata(fixture.0.join("browser/owner.lock")).unwrap().ino(), lock_inode);
        assert_eq!(fs::read(project).unwrap(), b"project state");
        replacement.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn widget_stream_retains_coalesced_subscription_snapshot_and_events() {
        let mut wire = Vec::new();
        for response in [
            WireResponse::WidgetSubscribed { window_id: "window".into() },
            WireResponse::WidgetEvent(WidgetEvent::Snapshot { sequence: 1, widgets: Vec::new() }),
            WireResponse::WidgetEvent(WidgetEvent::Snapshot { sequence: 2, widgets: Vec::new() }),
        ] {
            wire.extend(serde_json::to_vec(&response).unwrap());
            wire.push(b'\n');
        }
        // All three frames are deliberately returned by a single fill_buf.
        let mut reader = BufReader::with_capacity(wire.len(), wire.as_slice());
        assert!(matches!(
            read_widget_stream_frame(&mut reader).await.unwrap(),
            WireResponse::WidgetSubscribed { window_id } if window_id == "window"
        ));
        assert!(matches!(
            read_widget_stream_frame(&mut reader).await.unwrap(),
            WireResponse::WidgetEvent(WidgetEvent::Snapshot { sequence: 1, .. })
        ));
        let event = read_stream_frame(&mut reader, MAX_RESPONSE_FRAME).await.unwrap().unwrap();
        assert!(matches!(
            serde_json::from_slice::<WireResponse>(&event).unwrap(),
            WireResponse::WidgetEvent(WidgetEvent::Snapshot { sequence: 2, .. })
        ));
        assert!(read_stream_frame(&mut reader, MAX_RESPONSE_FRAME).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn widget_stream_frames_handle_split_boundaries_and_enforce_size() {
        for capacity in [1, 2, 3, 4, 5, 7] {
            let mut reader = BufReader::with_capacity(capacity, &b"abcd\nxy\nlast"[..]);
            for expected in [b"abcd".as_slice(), b"xy", b"last"] {
                assert_eq!(read_stream_frame(&mut reader, 4).await.unwrap().unwrap(), expected);
            }
            assert!(read_stream_frame(&mut reader, 4).await.unwrap().is_none());
            for oversized in [b"abcde\n".as_slice(), b"abcde"] {
                let mut reader = BufReader::with_capacity(capacity, oversized);
                assert_eq!(
                    read_stream_frame(&mut reader, 4).await.unwrap_err().kind(),
                    std::io::ErrorKind::InvalidData
                );
            }
        }
    }

    #[tokio::test]
    async fn widget_disconnected_and_owner_stopped_waits_release_all_slots() {
        let fixture = RuntimeFixture::new();
        let widgets = Arc::new(WidgetService::new(Arc::new(WidgetHerdr), Arc::new(WidgetHerdr)));
        let address = WidgetAddress {
            session_id: "daily".into(), endpoint_path: Some("/fixture/herdr.sock".into()),
            source_pane_id: None, locator: WidgetLocator::Tab { tab_id: "t1".into() }, space_check: None,
        };
        let spec_json = r#"{"prompt":"Choose","choices":[{"id":"one","label":"One"}]}"#.to_string();
        widgets.show(WidgetShowRequest {
            address: address.clone(), id: "waiting".into(), title: None,
            content: WidgetContentInput::Choices {
                sha256: format!("{:x}", Sha256::digest(spec_json.as_bytes())),
                spec_json, from: WidgetInputKind::File, name: None,
            },
            reopen: false, clear_selection: false,
        }).await.unwrap();
        let request = WidgetSelectionRequest {
            address, id: "waiting".into(), wait_seconds: Some(WIDGET_MAX_WAIT_SECONDS),
        };
        let service = fixture.service();
        let helper = Arc::new(BrowserHelperSupervisor::new(fixture.0.clone()));
        let mut capacity_request = request.clone();
        capacity_request.wait_seconds = Some(0);
        // A second full batch proves all eight slots became reusable, not
        // merely that a disconnect stopped one socket handler.
        for shutdown_owner in [false, true] {
            for _ in 0..2 {
                // A fresh receiver must not inherit an unseen false reset:
                // changed() is a notification, not a test of the stored bool.
                let (stopped, owner_stopped) = tokio::sync::watch::channel(false);
                let mut completed_early = 0;
                let mut clients = Vec::new();
                let mut peers = Vec::new();
                for _ in 0..WIDGET_MAX_SELECTION_WAITERS {
                    let (mut client, server) = UnixStream::pair().unwrap();
                    let mut frame = serde_json::to_vec(&WireRequest::WidgetSelection(request.clone())).unwrap();
                    frame.push(b'\n');
                    client.write_all(&frame).await.unwrap();
                    server.readable().await.unwrap();
                    let mut peer = Box::pin(serve_peer(
                        server, service.clone(), helper.clone(), widgets.clone(), owner_stopped.clone(),
                    ));
                    // Drive the already-readable request. Registration is
                    // verified below through public capacity behavior, not
                    // an assertion about this future's scheduling state.
                    if poll_fn(|cx| Poll::Ready(peer.as_mut().poll(cx))).await.is_ready() {
                        completed_early += 1;
                    }
                    clients.push(client);
                    peers.push(peer);
                }
                // This checks actual service registration rather than relying
                // on task scheduling, sleeps or socket response echoes.
                let capacity = widgets.selection(capacity_request.clone()).await;
                assert!(
                    matches!(&capacity, Err(error) if error.code == "widget_busy"),
                    "expected eight registered waits; capacity probe: {capacity:?}; \
                     {completed_early} peer handlers completed before cancellation"
                );
                if shutdown_owner {
                    stopped.send(true).unwrap();
                } else {
                    clients.clear();
                }
                for peer in peers {
                    timeout(Duration::from_secs(2), peer).await.unwrap();
                }
                drop(clients);
            }
        }
        // Zero-wait selection must work after the final cancellations too.
        let mut immediate = request;
        immediate.wait_seconds = Some(0);
        assert_eq!(widgets.selection(immediate).await.unwrap().status, WidgetSelectionStatus::Timeout);
    }

    #[tokio::test]
    async fn widget_observer_event_stream_closes_when_owner_exits() {
        let fixture = RuntimeFixture::new();
        let owner = BrowserRuntime::start(fixture.0.clone(), fixture.service(), fixture.widgets()).await.unwrap();
        let observer = BrowserRuntime::connect(fixture.0.clone(), fixture.service()).await.unwrap();
        let mut stream = observer.widget_events().await.unwrap();
        owner.shutdown().await.unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), stream.events.recv()).await.unwrap(),
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
        observer.shutdown().await.unwrap();
    }
}
