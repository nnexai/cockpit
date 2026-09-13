mod comments;
mod context;
mod context_media;
mod context_search;
mod projects;
mod requests;
mod review;
mod sources;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use cockpit_core::{
    CockpitService, HerdrAdapter, InspectionError, SessionChange, SessionSubscription,
    TerminalSession,
};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_host::{
    BrowserRuntime,
    browser_runtime::validate_browser_view_command,
    server::browser_view::{FrameConnection, decode_frame},
};
use cockpit_protocol::{
    browser::{
        BrowserFeedbackAckRequest, BrowserFeedbackImage, BrowserFeedbackImageRequest,
        BrowserFeedbackLookup, BrowserFeedbackRequest, BrowserFeedbackSendRequest,
        BrowserFeedbackSendResponse, BrowserRequest, BrowserResponse,
    },
    browser_view::{
        BrowserViewCommandRequest, BrowserViewCommandResponse, BrowserViewEvent,
        BrowserViewOpenRequest, BrowserViewSnapshot,
    },
    v1::{
        CockpitMode, ErrorResponse, FocusRequest, FocusResponse, ResourceMutationRequest,
        ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse,
        SessionStreamMessage, StatusResponse, TerminalCommand, TerminalOpenRequest,
        TerminalOwnershipState, TerminalStreamMessage,
    },
};
use serde::Serialize;
use tauri::{
    Manager, State,
    ipc::{Channel, InvokeResponseBody},
};
use tokio::{
    io::AsyncWriteExt,
    process::Command as TokioCommand,
    sync::{Notify, mpsc},
};
use uuid::Uuid;

const MAX_STREAMS: usize = 256;
const RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);
const MAX_TERMINAL_COMMAND_BYTES: usize = 96 * 1024;
const MAX_MUTATION_REQUEST_BYTES: usize = 64 * 1024;
const CLIPBOARD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[tauri::command]
async fn cockpit_clipboard_write(text: String) -> Result<(), ErrorResponse> {
    #[cfg(target_os = "linux")]
    {
        write_linux_clipboard(&text).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = text;
        Err(stream_error(
            "clipboard_unavailable",
            "The native clipboard adapter is unavailable on this platform",
        ))
    }
}

#[tauri::command]
async fn cockpit_clipboard_read() -> Result<String, ErrorResponse> {
    #[cfg(target_os = "linux")]
    {
        read_linux_clipboard().await
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(stream_error(
            "clipboard_unavailable",
            "The native clipboard adapter is unavailable on this platform",
        ))
    }
}

#[cfg(target_os = "linux")]
async fn write_linux_clipboard(text: &str) -> Result<(), ErrorResponse> {
    let mut process = TokioCommand::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            stream_error(
                "clipboard_unavailable",
                format!("Could not start wl-copy: {error}"),
            )
        })?;
    let result = tokio::time::timeout(CLIPBOARD_TIMEOUT, async {
        let mut stdin = process.stdin.take().ok_or_else(|| {
            stream_error("clipboard_unavailable", "wl-copy stdin was unavailable")
        })?;
        stdin.write_all(text.as_bytes()).await.map_err(|error| {
            stream_error(
                "clipboard_write_failed",
                format!("Could not write the clipboard: {error}"),
            )
        })?;
        drop(stdin);
        let status = process.wait().await.map_err(|error| {
            stream_error(
                "clipboard_write_failed",
                format!("Could not finish the clipboard write: {error}"),
            )
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(stream_error(
                "clipboard_write_failed",
                format!("wl-copy exited with {status}"),
            ))
        }
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = process.kill().await;
            Err(stream_error(
                "clipboard_write_timeout",
                "The native clipboard write exceeded 2 seconds",
            ))
        }
    }
}

#[cfg(target_os = "linux")]
async fn read_linux_clipboard() -> Result<String, ErrorResponse> {
    let output = tokio::time::timeout(
        CLIPBOARD_TIMEOUT,
        TokioCommand::new("wl-paste")
            .args(["--no-newline", "--type", "text/plain"])
            .output(),
    )
    .await
    .map_err(|_| {
        stream_error(
            "clipboard_read_timeout",
            "The native clipboard read exceeded 2 seconds",
        )
    })?
    .map_err(|error| {
        stream_error(
            "clipboard_unavailable",
            format!("Could not start wl-paste: {error}"),
        )
    })?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| {
            stream_error(
                "clipboard_read_failed",
                format!("The clipboard was not valid UTF-8: {error}"),
            )
        })
    } else {
        Err(stream_error(
            "clipboard_read_failed",
            format!("wl-paste exited with {}", output.status),
        ))
    }
}

/// A cancellation/cleanup handle shared with a stream relay task.
struct StreamControl {
    cancelled: AtomicBool,
    abort: Mutex<Option<tokio::task::AbortHandle>>,
    release: Mutex<Option<mpsc::Sender<TerminalCommand>>>,
    cancel_notify: Option<Arc<Notify>>,
    abort_on_cancel: bool,
}

impl StreamControl {
    fn new(release: Option<mpsc::Sender<TerminalCommand>>) -> Arc<Self> {
        Self::with_cancel_notify(release, None, true)
    }

    fn new_browser() -> Arc<Self> {
        Self::with_cancel_notify(None, Some(Arc::new(Notify::new())), false)
    }

    fn with_cancel_notify(
        release: Option<mpsc::Sender<TerminalCommand>>,
        cancel_notify: Option<Arc<Notify>>,
        abort_on_cancel: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            abort: Mutex::new(None),
            release: Mutex::new(release),
            cancel_notify,
            abort_on_cancel,
        })
    }

    fn set_abort(&self, abort: tokio::task::AbortHandle) {
        if self.cancelled.load(Ordering::Acquire) {
            if self.abort_on_cancel {
                abort.abort();
            }
            return;
        }
        *self.abort.lock().expect("stream control lock poisoned") = Some(abort);
        if self.cancelled.load(Ordering::Acquire)
            && self.abort_on_cancel
            && let Some(abort) = self
                .abort
                .lock()
                .expect("stream control lock poisoned")
                .take()
        {
            abort.abort();
        }
    }

    async fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(notify) = &self.cancel_notify {
            notify.notify_one();
        }
        let release = self
            .release
            .lock()
            .expect("stream control lock poisoned")
            .take();
        if let Some(release) = release {
            let _ =
                tokio::time::timeout(RELEASE_TIMEOUT, release.send(TerminalCommand::Release)).await;
        }
        if self.abort_on_cancel
            && let Some(abort) = self
                .abort
                .lock()
                .expect("stream control lock poisoned")
                .take()
        {
            abort.abort();
        }
    }
}

enum StreamEntry {
    Session {
        control: Arc<StreamControl>,
    },
    Terminal {
        control: Arc<StreamControl>,
        commands: mpsc::Sender<TerminalCommand>,
    },
    BrowserView {
        control: Arc<StreamControl>,
        acknowledgements: mpsc::Sender<u64>,
    },
}

/// Process-local bounded registry for live native streams.
#[derive(Clone)]
struct StreamRegistry {
    entries: Arc<Mutex<HashMap<String, StreamEntry>>>,
}

impl StreamRegistry {
    fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn allocate(&self, entry: StreamEntry) -> Result<String, ErrorResponse> {
        let mut entries = self.entries.lock().expect("stream registry lock poisoned");
        if entries.len() >= MAX_STREAMS {
            return Err(ErrorResponse {
                code: "stream_limit".to_owned(),
                message: "The maximum number of active streams has been reached".to_owned(),
            });
        }
        let id = loop {
            let candidate = Uuid::new_v4().simple().to_string();
            if !entries.contains_key(&candidate) {
                break candidate;
            }
        };
        entries.insert(id.clone(), entry);
        Ok(id)
    }

    fn terminal_commands(&self, stream_id: &str) -> Option<mpsc::Sender<TerminalCommand>> {
        let entries = self.entries.lock().expect("stream registry lock poisoned");
        match entries.get(stream_id) {
            Some(StreamEntry::Terminal { commands, .. }) => Some(commands.clone()),
            _ => None,
        }
    }

    fn browser_view_acknowledgements(&self, stream_id: &str) -> Option<mpsc::Sender<u64>> {
        let entries = self.entries.lock().expect("stream registry lock poisoned");
        match entries.get(stream_id) {
            Some(StreamEntry::BrowserView {
                acknowledgements, ..
            }) => Some(acknowledgements.clone()),
            _ => None,
        }
    }

    fn complete(&self, stream_id: &str) {
        let entry = self
            .entries
            .lock()
            .expect("stream registry lock poisoned")
            .remove(stream_id);
        if let Some(entry) = entry {
            match entry {
                StreamEntry::Session { control }
                | StreamEntry::Terminal { control, .. }
                | StreamEntry::BrowserView { control, .. } => {
                    control.cancelled.store(true, Ordering::Release);
                }
            }
        }
    }

    async fn cancel(&self, stream_id: &str) {
        let entry = self
            .entries
            .lock()
            .expect("stream registry lock poisoned")
            .remove(stream_id);
        if let Some(entry) = entry {
            match entry {
                StreamEntry::Session { control }
                | StreamEntry::Terminal { control, .. }
                | StreamEntry::BrowserView { control, .. } => {
                    control.cancel().await;
                }
            }
        }
    }
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a service failure to the shared native error envelope.
fn inspection_error_response(error: InspectionError) -> ErrorResponse {
    ErrorResponse {
        code: error.code,
        message: error.message,
    }
}

fn stream_error(code: &str, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse {
        code: code.to_owned(),
        message: message.into(),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod clipboard_tests {
    use super::{read_linux_clipboard, write_linux_clipboard};

    #[tokio::test]
    #[ignore = "requires a run-owned Wayland display; run explicitly for native integration proof"]
    async fn native_clipboard_roundtrip_preserves_unicode_and_newlines() {
        let expected = "Cockpit αβγ\nline two\n✓ 终端";
        write_linux_clipboard(expected)
            .await
            .expect("wl-copy should accept text");
        let actual = read_linux_clipboard()
            .await
            .expect("wl-paste should return text");
        assert_eq!(actual, expected);
    }
}

fn advance_sequence(sequence: &mut u32, generation: &mut u32) -> Result<(), ErrorResponse> {
    if *sequence == u32::MAX {
        *generation = generation
            .checked_add(1)
            .ok_or_else(|| stream_error("sequence_overflow", "The stream generation overflowed"))?;
        *sequence = 1;
    } else {
        *sequence += 1;
    }
    Ok(())
}

/// Return the shared status response to the native frontend.
#[tauri::command]
async fn cockpit_status(
    service: State<'_, CockpitService>,
) -> Result<StatusResponse, ErrorResponse> {
    Ok(service.status().await)
}

#[tauri::command]
async fn cockpit_browser_action(
    request: BrowserRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<BrowserResponse, ErrorResponse> {
    runtime
        .execute(request)
        .await
        .map_err(inspection_error_response)
}
#[tauri::command]
async fn cockpit_browser_feedback(
    request: BrowserFeedbackRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<BrowserFeedbackLookup, ErrorResponse> {
    runtime
        .feedback(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_browser_feedback_ack(
    request: BrowserFeedbackAckRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<cockpit_protocol::browser_feedback::BrowserFeedbackAck, ErrorResponse> {
    runtime
        .acknowledge_feedback(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_browser_feedback_image(
    request: BrowserFeedbackImageRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<BrowserFeedbackImage, ErrorResponse> {
    runtime
        .feedback_image(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_browser_feedback_send(
    request: BrowserFeedbackSendRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<BrowserFeedbackSendResponse, ErrorResponse> {
    runtime
        .send_feedback(request)
        .await
        .map_err(inspection_error_response)
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BrowserViewChannelMessage {
    Event {
        event: BrowserViewEvent,
    },
    Frame {
        descriptor: cockpit_protocol::browser_view::BrowserViewFrameDescriptor,
    },
    Error {
        code: String,
        message: String,
    },
}

fn send_browser_view_message(
    channel: &Channel<InvokeResponseBody>,
    message: BrowserViewChannelMessage,
) -> bool {
    let Ok(json) = serde_json::to_string(&message) else {
        return false;
    };
    channel.send(InvokeResponseBody::Json(json)).is_ok()
}

#[derive(Clone, Debug, Serialize)]
struct BrowserViewOpenNativeResponse {
    snapshot: BrowserViewSnapshot,
    first_frame: cockpit_protocol::browser_view::BrowserViewFrameDescriptor,
}

#[tauri::command]
async fn cockpit_browser_draft_recovery(
    request: cockpit_protocol::browser_view::BrowserDraftRecoveryRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<cockpit_protocol::browser_view::BrowserViewCommandOutcome, ErrorResponse> {
    runtime.browser_draft_recovery(request).await.map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_browser_view_open(
    request: BrowserViewOpenRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<BrowserViewOpenNativeResponse, ErrorResponse> {
    request
        .validate()
        .map_err(|message| stream_error("invalid_browser_view_open", message))?;
    let opened = runtime
        .open_browser_view(request)
        .await
        .map_err(inspection_error_response)?;
    Ok(BrowserViewOpenNativeResponse {
        snapshot: opened.snapshot,
        first_frame: opened.first_frame,
    })
}

#[tauri::command]
async fn cockpit_browser_view_command(
    request: BrowserViewCommandRequest,
    runtime: State<'_, Arc<BrowserRuntime>>,
) -> Result<BrowserViewCommandResponse, ErrorResponse> {
    validate_browser_view_command(&request)
        .map_err(|error| stream_error(&error.code, error.message))?;
    runtime
        .browser_view_command(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_browser_view_subscribe(
    view_id: String,
    stream_epoch: u64,
    channel: Channel<InvokeResponseBody>,
    runtime: State<'_, Arc<BrowserRuntime>>,
    registry: State<'_, StreamRegistry>,
) -> Result<String, ErrorResponse> {
    let subscription = runtime
        .browser_view_events(&view_id)
        .await
        .map_err(inspection_error_response)?;
    if subscription.snapshot.identity.stream_epoch != stream_epoch {
        return Err(stream_error(
            "stale_browser_view",
            "Browser view stream epoch is stale",
        ));
    }
    let initial_snapshot = subscription.snapshot.clone();
    let grant = initial_snapshot.frame_grant.clone().ok_or_else(|| {
        stream_error(
            "browser_frame_unavailable",
            "Browser view has no frame grant",
        )
    })?;
    let endpoint = runtime
        .browser_view_frame_endpoint(&grant)
        .await
        .map_err(inspection_error_response)?;
    let (ack_tx, mut ack_rx) = mpsc::channel(8);
    let control = StreamControl::new_browser();
    let stream_id = registry.allocate(StreamEntry::BrowserView {
        control: Arc::clone(&control),
        acknowledgements: ack_tx,
    })?;
    let task_registry = registry.inner().clone();
    let task_stream_id = stream_id.clone();
    let task_control = Arc::clone(&control);
    let task_runtime = runtime.inner().clone();
    let task_view_id = initial_snapshot.identity.view_id.clone();
    let cancel_notify = task_control
        .cancel_notify
        .clone()
        .expect("browser stream cancellation notification");
    let task = tokio::spawn(async move {
        let mut events = subscription.events;
        let mut frame = tokio::select! {
            _ = cancel_notify.notified() => {
                drop(events);
                drop(ack_rx);
                task_registry.complete(&task_stream_id);
                let _ = task_runtime.browser_view_detach(&task_view_id).await;
                return;
            }
            result = FrameConnection::connect(&endpoint, &grant.grant) => match result {
                Ok(frame) => frame,
                Err(error) => {
                    let _ = send_browser_view_message(
                        &channel,
                        BrowserViewChannelMessage::Error {
                            code: error.code.to_owned(),
                            message: error.message.to_owned(),
                        },
                    );
                    drop(events);
                    drop(ack_rx);
                    task_registry.complete(&task_stream_id);
                    let _ = task_runtime.browser_view_detach(&task_view_id).await;
                    return;
                }
            },
        };
        let mut target_id = initial_snapshot
            .displayed_target_id
            .clone()
            .unwrap_or_default();
        let metadata = cockpit_protocol::browser_view::BrowserViewEventMetadata {
            view_id: initial_snapshot.identity.view_id.clone(),
            stream_epoch,
            metadata_sequence: initial_snapshot.metadata_sequence,
        };
        if !send_browser_view_message(
            &channel,
            BrowserViewChannelMessage::Event {
                event: BrowserViewEvent::Attached {
                    metadata,
                    snapshot: initial_snapshot.clone(),
                },
            },
        ) {
            drop(frame);
            drop(events);
            drop(ack_rx);
            task_registry.complete(&task_stream_id);
            let _ = task_runtime.browser_view_detach(&task_view_id).await;
            return;
        }
        let mut outstanding = None;
        while !task_control.cancelled.load(Ordering::Acquire) {
            tokio::select! {
                _ = cancel_notify.notified() => break,
                event = events.recv() => match event {
                    Ok(event) => {
                        if let BrowserViewEvent::Attached { snapshot, .. } = &event { target_id = snapshot.displayed_target_id.clone().unwrap_or(target_id); }
                        if let BrowserViewEvent::TargetsChanged { displayed_target_id, .. } = &event { target_id = displayed_target_id.clone().unwrap_or(target_id); }
                        if !send_browser_view_message(&channel, BrowserViewChannelMessage::Event { event }) { break; }
                    }
                    Err(_) => {
                        let _ = send_browser_view_message(
                            &channel,
                            BrowserViewChannelMessage::Error {
                                code: "browser_metadata_closed".into(),
                                message: "Browser metadata stream closed".into(),
                            },
                        );
                        break;
                    }
                },
                ack = ack_rx.recv() => {
                    let Some(sequence) = ack else { break; };
                    if outstanding == Some(sequence) {
                        if frame.send_credit("ack", sequence).await.is_err() { break; }
                        outstanding = None;
                    }
                }
                incoming = frame.recv() => match incoming {
                    Ok(Some(payload)) => {
                        let packet = match decode_frame(&payload, &target_id, stream_epoch) {
                            Ok(packet) => packet,
                            Err(error) => {
                                let _ = send_browser_view_message(
                                    &channel,
                                    BrowserViewChannelMessage::Error {
                                        code: error.code.to_owned(),
                                        message: error.message.to_owned(),
                                    },
                                );
                                break;
                            }
                        };
                        if outstanding.is_some() {
                            if frame.send_credit("discard", packet.descriptor.frame_sequence).await.is_err() { break; }
                            continue;
                        }
                        let sequence = packet.descriptor.frame_sequence;
                        let descriptor = packet.descriptor;
                        let jpeg = packet.jpeg;
                        outstanding = Some(sequence);
                        if !send_browser_view_message(
                            &channel,
                            BrowserViewChannelMessage::Frame { descriptor },
                        ) {
                            outstanding = None;
                            let _ = frame.send_credit("discard", sequence).await;
                            break;
                        }
                        if channel.send(InvokeResponseBody::Raw(jpeg)).is_err() {
                            outstanding = None;
                            let _ = frame.send_credit("discard", sequence).await;
                            break;
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
        if let Some(sequence) = outstanding {
            let _ = frame.send_credit("discard", sequence).await;
        }
        drop(frame);
        drop(events);
        drop(ack_rx);
        task_registry.complete(&task_stream_id);
        let _ = task_runtime.browser_view_detach(&task_view_id).await;
    });
    control.set_abort(task.abort_handle());
    Ok(stream_id)
}

#[tauri::command]
async fn cockpit_browser_view_frame_ack(
    stream_id: String,
    frame_sequence: u64,
    registry: State<'_, StreamRegistry>,
) -> Result<(), ErrorResponse> {
    let Some(acknowledgements) = registry.browser_view_acknowledgements(&stream_id) else {
        return Err(stream_error(
            "stream_not_found",
            "The browser view stream is not active",
        ));
    };
    acknowledgements.try_send(frame_sequence).map_err(|_| {
        stream_error(
            "stream_closed",
            "The browser view stream is not accepting frame credit",
        )
    })
}

#[tauri::command]
async fn cockpit_sessions(
    service: State<'_, CockpitService>,
) -> Result<SessionListResponse, ErrorResponse> {
    service.sessions().await.map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_session_snapshot(
    session_id: String,
    service: State<'_, CockpitService>,
) -> Result<SessionSnapshotResponse, ErrorResponse> {
    service
        .session_snapshot(&session_id)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_focus(
    session_id: String,
    request: FocusRequest,
    service: State<'_, CockpitService>,
) -> Result<FocusResponse, ErrorResponse> {
    service
        .focus(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
async fn cockpit_mutate(
    session_id: String,
    request: ResourceMutationRequest,
    service: State<'_, CockpitService>,
) -> Result<ResourceMutationResponse, ErrorResponse> {
    let encoded = serde_json::to_vec(&request)
        .map_err(|_| stream_error("invalid_mutation_request", "Invalid mutation request"))?;
    if encoded.len() > MAX_MUTATION_REQUEST_BYTES {
        return Err(stream_error(
            "mutation_request_too_large",
            "Mutation request exceeds the 64 KiB limit",
        ));
    }
    service
        .mutate(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

struct SessionRelayContext {
    stream_id: String,
    session_id: String,
    initial: SessionSnapshotResponse,
    subscription: SessionSubscription,
    channel: Channel<SessionStreamMessage>,
    service: CockpitService,
    control: Arc<StreamControl>,
    registry: StreamRegistry,
}

async fn relay_session(context: SessionRelayContext) {
    let SessionRelayContext {
        stream_id,
        session_id,
        initial,
        subscription,
        channel,
        service,
        control,
        registry,
    } = context;
    let mut sequence = 1;
    let mut generation = 1;
    if channel
        .send(SessionStreamMessage::Snapshot {
            session_id: session_id.clone(),
            generation,
            sequence,
            snapshot: initial,
        })
        .is_err()
    {
        registry.complete(&stream_id);
        return;
    }
    if advance_sequence(&mut sequence, &mut generation).is_err() {
        registry.complete(&stream_id);
        return;
    }
    let snapshot = match service.session_snapshot(&session_id).await {
        Ok(snapshot) if snapshot.session_id == session_id => snapshot,
        Ok(_) => {
            let _ = channel.send(SessionStreamMessage::Stale {
                session_id: session_id.clone(),
                generation,
                sequence,
                code: "session_snapshot_mismatch".to_owned(),
                message: "Session snapshot unavailable".to_owned(),
            });
            registry.complete(&stream_id);
            return;
        }
        Err(error) => {
            let _ = channel.send(SessionStreamMessage::Stale {
                session_id: session_id.clone(),
                generation,
                sequence,
                code: error.code,
                message: error.message,
            });
            registry.complete(&stream_id);
            return;
        }
    };
    if channel
        .send(SessionStreamMessage::Snapshot {
            session_id: session_id.clone(),
            generation,
            sequence,
            snapshot,
        })
        .is_err()
        || advance_sequence(&mut sequence, &mut generation).is_err()
    {
        registry.complete(&stream_id);
        return;
    }

    let mut messages = subscription.messages;
    while !control.cancelled.load(Ordering::Acquire) {
        let Some(change) = messages.recv().await else {
            let _ = channel.send(SessionStreamMessage::Disconnected {
                session_id: session_id.clone(),
                generation,
                sequence,
                code: "subscription_closed".to_owned(),
                message: "The session stream disconnected".to_owned(),
            });
            break;
        };
        let (message, starts_new_generation) = match change {
            SessionChange::Changed => match service.session_snapshot(&session_id).await {
                Ok(snapshot) if snapshot.session_id == session_id => (
                    SessionStreamMessage::Snapshot {
                        session_id: session_id.clone(),
                        generation,
                        sequence,
                        snapshot,
                    },
                    false,
                ),
                Ok(_) => (
                    SessionStreamMessage::Stale {
                        session_id: session_id.clone(),
                        generation,
                        sequence,
                        code: "session_snapshot_mismatch".to_owned(),
                        message: "Session snapshot unavailable".to_owned(),
                    },
                    true,
                ),
                Err(error) => (
                    SessionStreamMessage::Stale {
                        session_id: session_id.clone(),
                        generation,
                        sequence,
                        code: error.code,
                        message: error.message,
                    },
                    true,
                ),
            },
            SessionChange::Stale { code, message } => (
                SessionStreamMessage::Stale {
                    session_id: session_id.clone(),
                    generation,
                    sequence,
                    code,
                    message,
                },
                true,
            ),
            SessionChange::Disconnected { code, message } => (
                SessionStreamMessage::Disconnected {
                    session_id: session_id.clone(),
                    generation,
                    sequence,
                    code,
                    message,
                },
                true,
            ),
        };
        if channel.send(message).is_err() {
            break;
        }
        if starts_new_generation {
            let Some(next_generation) = generation.checked_add(1) else {
                break;
            };
            generation = next_generation;
            sequence = 1;
        } else if advance_sequence(&mut sequence, &mut generation).is_err() {
            let _ = channel.send(SessionStreamMessage::Stale {
                session_id: session_id.clone(),
                generation,
                sequence,
                code: "sequence_overflow".to_owned(),
                message: "The stream sequence overflowed".to_owned(),
            });
            break;
        }
    }
    registry.complete(&stream_id);
}

#[tauri::command]
async fn cockpit_session_subscribe(
    session_id: String,
    channel: Channel<SessionStreamMessage>,
    service: State<'_, CockpitService>,
    registry: State<'_, StreamRegistry>,
) -> Result<String, ErrorResponse> {
    let initial = service
        .session_snapshot(&session_id)
        .await
        .map_err(inspection_error_response)?;
    if initial.session_id != session_id {
        return Err(stream_error(
            "session_snapshot_mismatch",
            "Session snapshot unavailable",
        ));
    }
    let subscription = service
        .subscribe_session(&session_id, &initial)
        .await
        .map_err(inspection_error_response)?;
    let control = StreamControl::new(None);
    let stream_id = registry.allocate(StreamEntry::Session {
        control: Arc::clone(&control),
    })?;
    let task_registry = registry.inner().clone();
    let returned_stream_id = stream_id.clone();
    let task = tokio::spawn(relay_session(SessionRelayContext {
        stream_id,
        session_id,
        initial,
        subscription,
        channel,
        service: service.inner().clone(),
        control: control.clone(),
        registry: task_registry,
    }));
    control.set_abort(task.abort_handle());
    Ok(returned_stream_id)
}

fn localize_terminal_message(
    message: TerminalStreamMessage,
    stream_id: &str,
) -> TerminalStreamMessage {
    match message {
        TerminalStreamMessage::MouseMode {
            session_id,
            pane_id,
            enabled,
            ..
        } => TerminalStreamMessage::MouseMode {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            enabled,
        },
        TerminalStreamMessage::Ownership {
            session_id,
            pane_id,
            state,
            message,
            ..
        } => TerminalStreamMessage::Ownership {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            state,
            message,
        },
        TerminalStreamMessage::Frame {
            session_id,
            pane_id,
            seq,
            encoding,
            width,
            height,
            full,
            bytes,
            ..
        } => TerminalStreamMessage::Frame {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            seq,
            encoding,
            width,
            height,
            full,
            bytes,
        },
        TerminalStreamMessage::Closed {
            session_id,
            pane_id,
            reason,
            ..
        } => TerminalStreamMessage::Closed {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            reason,
        },
        TerminalStreamMessage::Disconnected {
            session_id,
            pane_id,
            code,
            message,
            ..
        } => TerminalStreamMessage::Disconnected {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            code,
            message,
        },
        TerminalStreamMessage::Error {
            session_id,
            pane_id,
            code,
            message,
            ..
        } => TerminalStreamMessage::Error {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            code,
            message,
        },
    }
}
fn terminal_error(
    request: &TerminalOpenRequest,
    stream_id: &str,
    code: &str,
    message: &str,
) -> TerminalStreamMessage {
    TerminalStreamMessage::Error {
        session_id: request.session_id.clone(),
        pane_id: request.pane_id.clone(),
        stream_id: stream_id.to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn decimal_sequence(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn validate_terminal_message(
    message: &TerminalStreamMessage,
    request: &TerminalOpenRequest,
    herdr_stream_id: &str,
    last_seq: &mut Option<u64>,
    has_baseline: &mut bool,
) -> Result<(), (&'static str, &'static str)> {
    let (session_id, pane_id, stream_id) = match message {
        TerminalStreamMessage::MouseMode {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Ownership {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Frame {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Closed {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Disconnected {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Error {
            session_id,
            pane_id,
            stream_id,
            ..
        } => (session_id, pane_id, stream_id),
    };
    if session_id != &request.session_id || pane_id != &request.pane_id {
        return Err(("terminal_frame_invalid", "Terminal frame resource mismatch"));
    }
    if stream_id != herdr_stream_id {
        return Err(("terminal_frame_invalid", "Terminal frame stream mismatch"));
    }
    if let TerminalStreamMessage::Frame {
        seq,
        encoding,
        width: _,
        height: _,
        full,
        bytes,
        ..
    } = message
    {
        if encoding != "ansi" || BASE64.decode(bytes.as_bytes()).is_err() {
            return Err(("terminal_frame_invalid", "Invalid terminal frame"));
        }
        let Some(number) = decimal_sequence(seq) else {
            return Err(("terminal_sequence_error", "Invalid terminal sequence"));
        };
        if !*has_baseline && !*full {
            return Err((
                "terminal_sequence_error",
                "Incremental terminal frame has no full-frame baseline",
            ));
        }
        if let Some(previous) = *last_seq
            && number != previous.saturating_add(1)
        {
            return Err((
                "terminal_sequence_error",
                "Terminal sequence is not consecutive",
            ));
        }
        *last_seq = Some(number);
        *has_baseline = true;
    }
    Ok(())
}

#[tauri::command]
async fn cockpit_terminal_open(
    request: TerminalOpenRequest,
    channel: Channel<TerminalStreamMessage>,
    service: State<'_, CockpitService>,
    registry: State<'_, StreamRegistry>,
) -> Result<String, ErrorResponse> {
    let terminal = service
        .open_terminal(&request)
        .await
        .map_err(inspection_error_response)?;
    let TerminalSession {
        stream_id: herdr_stream_id,
        messages,
        commands,
    } = terminal;
    let control = StreamControl::new(Some(commands.clone()));
    let stream_id = registry.allocate(StreamEntry::Terminal {
        control: Arc::clone(&control),
        commands: commands.clone(),
    })?;
    let task_registry = registry.inner().clone();
    let task_stream_id = stream_id.clone();
    let task_control = control.clone();
    let task_request = request.clone();
    let task = tokio::spawn(async move {
        let mut messages = messages;
        let mut last_seq = None;
        let mut has_baseline = false;
        let mut terminal_end = false;
        while !task_control.cancelled.load(Ordering::Acquire) {
            let Some(message) = messages.recv().await else {
                if !terminal_end {
                    let _ = channel.send(terminal_error(
                        &task_request,
                        &task_stream_id,
                        "terminal_disconnected",
                        "Terminal stream disconnected",
                    ));
                }
                break;
            };
            if let Err((code, message_text)) = validate_terminal_message(
                &message,
                &task_request,
                &herdr_stream_id,
                &mut last_seq,
                &mut has_baseline,
            ) {
                let _ = channel.send(terminal_error(
                    &task_request,
                    &task_stream_id,
                    code,
                    message_text,
                ));
                break;
            }
            terminal_end = matches!(
                &message,
                TerminalStreamMessage::Closed { .. }
                    | TerminalStreamMessage::Disconnected { .. }
                    | TerminalStreamMessage::Error { .. }
                    | TerminalStreamMessage::Ownership {
                        state: TerminalOwnershipState::Lost | TerminalOwnershipState::Conflict,
                        ..
                    }
            );
            if channel
                .send(localize_terminal_message(message, &task_stream_id))
                .is_err()
            {
                break;
            }
            if terminal_end {
                break;
            }
        }
        let _ = tokio::time::timeout(RELEASE_TIMEOUT, task_control_release(&task_control)).await;
        task_registry.complete(&task_stream_id);
    });
    control.set_abort(task.abort_handle());
    Ok(stream_id)
}

async fn task_control_release(control: &StreamControl) {
    let release = control
        .release
        .lock()
        .expect("stream control lock poisoned")
        .take();
    if let Some(release) = release {
        let _ = release.send(TerminalCommand::Release).await;
    }
}

#[tauri::command]
async fn cockpit_terminal_command(
    stream_id: String,
    command: TerminalCommand,
    registry: State<'_, StreamRegistry>,
) -> Result<(), ErrorResponse> {
    command
        .validate()
        .map_err(|message| stream_error("invalid_terminal_command", message))?;
    let encoded = serde_json::to_vec(&command)
        .map_err(|_| stream_error("invalid_terminal_command", "Invalid terminal command"))?;
    if encoded.len() > MAX_TERMINAL_COMMAND_BYTES {
        return Err(stream_error(
            "terminal_command_too_large",
            "Terminal command exceeds the 96 KiB limit",
        ));
    }
    let Some(commands) = registry.terminal_commands(&stream_id) else {
        return Err(stream_error(
            "stream_not_found",
            "The terminal stream is not active",
        ));
    };
    commands
        .send(command)
        .await
        .map_err(|_| stream_error("stream_closed", "The terminal stream is closed"))
}

#[tauri::command]
async fn cockpit_stream_cancel(
    stream_id: String,
    registry: State<'_, StreamRegistry>,
) -> Result<(), ErrorResponse> {
    registry.cancel(&stream_id).await;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let window_config = cockpit_core::config::load_window_configuration(None)
        .unwrap_or_else(|error| panic!("failed to load native window configuration: {error}"));
    let config =
        HerdrCliConfig::from_options(None, None, None).expect("failed to load Herdr configuration");
    let inspector = Arc::new(HerdrCliAdapter::new(config).with_server_autostart());
    let startup_inspector = Arc::clone(&inspector);
    let project_config = cockpit_core::config::load_project_configuration(None, None)
        .expect("failed to load project configuration");
    let sources = Arc::new(
        cockpit_core::sources::SourceService::new(
            &project_config,
            cockpit_providers::configured_providers(&project_config)
                .expect("invalid configured source providers"),
        )
        .expect("failed to initialize source cache"),
    );
    let project_service =
        cockpit_core::projects::ProjectService::new(project_config.clone(), inspector.clone())
            .expect("failed to initialize project operations")
            .with_sources(sources.clone());
    let browser_config = cockpit_core::config::load_browser_configuration(None)
        .expect("failed to load browser configuration");
    let paste_adapter = inspector.paste_adapter();
    let browser_service = Arc::new(
        cockpit_core::browser::BrowserService::new(
            browser_config,
            std::path::PathBuf::from(&project_config.state_root),
            inspector.clone(),
        )
        .expect("failed to initialize browser service")
        .with_paste_adapter(paste_adapter),
    );
    let browser_runtime = Arc::new(
        tauri::async_runtime::block_on(BrowserRuntime::start(
            std::path::PathBuf::from(&project_config.state_root),
            browser_service,
        ))
        .expect("failed to initialize browser runtime"),
    );
    let service =
        CockpitService::new(CockpitMode::Normal, inspector.clone()).with_projects(project_service);
    let shutdown_projects = service
        .projects()
        .expect("project operations configured")
        .clone();
    let contexts = cockpit_core::context::ContextService::new(
        project_config.clone(),
        inspector.extension_adapter(),
        shutdown_projects.clone(),
    );
    let contexts = contexts.with_sources(sources);
    let service = service.with_contexts(contexts);
    let reviews = cockpit_core::review::ReviewService::new(
        project_config.clone(),
        inspector.extension_adapter(),
        service
            .contexts()
            .expect("context operations configured")
            .clone(),
    )
    .expect("failed to initialize review operations");
    let service = service.with_reviews(reviews);
    let comments = cockpit_core::comments::CommentsService::new(
        project_config,
        service
            .contexts()
            .expect("context operations configured")
            .clone(),
    )
    .unwrap_or_else(|error| panic!("failed to initialize comment operations: {error}"))
    .with_paste_adapter(inspector.paste_adapter())
    .with_reviews(
        service
            .reviews()
            .expect("review operations configured")
            .clone(),
    );
    let service = service.with_comments(comments);
    tauri::Builder::default()
        .manage(service)
        .manage(browser_runtime.clone())
        .manage(StreamRegistry::new())
        .setup(move |app| {
            let main_window = app
                .get_webview_window("main")
                .expect("main window is missing from the Tauri configuration");
            main_window
                .set_zoom(window_config.scale_factor)
                .expect("failed to apply configured window scale factor");
            main_window
                .set_decorations(window_config.decorations)
                .expect("failed to apply configured window decorations");
            main_window
                .show()
                .expect("failed to show the configured native window");
            tauri::async_runtime::spawn(async move {
                if let Err(error) = startup_inspector.inspect().await {
                    eprintln!("failed to initialize Herdr: {error}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            projects::cockpit_resolve_workspace_defaults,
            projects::cockpit_project_configuration,
            projects::cockpit_repositories,
            projects::cockpit_workspace_plan,
            projects::cockpit_workspace_start,
            projects::cockpit_workspace_operation,
            projects::cockpit_workspace_resume,
            projects::cockpit_workspace_cancel,
            projects::cockpit_workspace_reconcile,
            projects::cockpit_workspace_teardown_preview,
            projects::cockpit_workspace_teardown_execute,
            projects::cockpit_workspace_teardown_recoveries,
            context::cockpit_pane_presentation,
            context::cockpit_context_directory,
            context::cockpit_context_document,
            context::cockpit_context_open,
            context_search::cockpit_context_search,
            context_search::cockpit_context_snapshot,
            context_search::cockpit_context_invalidate,
            context_media::cockpit_context_media,
            sources::cockpit_source_import,
            sources::cockpit_source_refresh,
            sources::cockpit_source_list,
            review::cockpit_review_snapshot,
            review::cockpit_review_file,
            review::cockpit_review_open,
            comments::cockpit_comments_list,
            comments::cockpit_comments_batch,
            comments::cockpit_comments_upsert,
            comments::cockpit_comments_remove,
            comments::cockpit_comments_attach,
            comments::cockpit_comments_discard,
            cockpit_status,
            cockpit_browser_action,
            cockpit_browser_view_open,
            cockpit_browser_draft_recovery,
            cockpit_browser_view_command,
            cockpit_browser_view_subscribe,
            cockpit_browser_view_frame_ack,
            comments::cockpit_comments_preview,
            comments::cockpit_comments_paste_prepare,
            comments::cockpit_comments_paste_send,
            comments::cockpit_comments_paste_mark_pasted,
            cockpit_browser_feedback,
            cockpit_browser_feedback_ack,
            cockpit_browser_feedback_image,
            cockpit_browser_feedback_send,
            cockpit_sessions,
            cockpit_session_snapshot,
            cockpit_focus,
            cockpit_mutate,
            cockpit_session_subscribe,
            cockpit_terminal_open,
            cockpit_terminal_command,
            cockpit_stream_cancel,
            cockpit_clipboard_read,
            cockpit_clipboard_write
        ])
        .build(tauri::generate_context!())
        .expect("error while building Cockpit Tauri application")
        .run(move |app, event| {
            if let tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
                let projects = shutdown_projects.clone();
                let browser_runtime = browser_runtime.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    projects.shutdown().await;
                    let _ = browser_runtime.shutdown().await;
                    app.exit(0);
                });
            }
        });
}
