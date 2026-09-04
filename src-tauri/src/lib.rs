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
use cockpit_protocol::v1::{
    CockpitMode, ErrorResponse, FocusRequest, FocusResponse, ResourceMutationRequest,
    ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse, SessionStreamMessage,
    StatusResponse, TerminalCommand, TerminalOpenRequest, TerminalOwnershipState,
    TerminalStreamMessage,
};
use tauri::{State, ipc::Channel};
use tokio::sync::mpsc;
use uuid::Uuid;

const MAX_STREAMS: usize = 256;
const RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);
const MAX_TERMINAL_COMMAND_BYTES: usize = 96 * 1024;
const MAX_MUTATION_REQUEST_BYTES: usize = 64 * 1024;

/// A cancellation/cleanup handle shared with a stream relay task.
struct StreamControl {
    cancelled: AtomicBool,
    abort: Mutex<Option<tokio::task::AbortHandle>>,
    release: Mutex<Option<mpsc::Sender<TerminalCommand>>>,
}

impl StreamControl {
    fn new(release: Option<mpsc::Sender<TerminalCommand>>) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            abort: Mutex::new(None),
            release: Mutex::new(release),
        })
    }

    fn set_abort(&self, abort: tokio::task::AbortHandle) {
        if self.cancelled.load(Ordering::Acquire) {
            abort.abort();
            return;
        }
        *self.abort.lock().expect("stream control lock poisoned") = Some(abort);
        if self.cancelled.load(Ordering::Acquire)
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
        let release = self
            .release
            .lock()
            .expect("stream control lock poisoned")
            .take();
        if let Some(release) = release {
            let _ =
                tokio::time::timeout(RELEASE_TIMEOUT, release.send(TerminalCommand::Release)).await;
        }
        if let Some(abort) = self
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

    fn complete(&self, stream_id: &str) {
        let entry = self
            .entries
            .lock()
            .expect("stream registry lock poisoned")
            .remove(stream_id);
        if let Some(entry) = entry {
            match entry {
                StreamEntry::Session { control } | StreamEntry::Terminal { control, .. } => {
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
                StreamEntry::Session { control } | StreamEntry::Terminal { control, .. } => {
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
        TerminalStreamMessage::Graphics {
            session_id,
            pane_id,
            revision,
            bytes,
            ..
        } => TerminalStreamMessage::Graphics {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            revision,
            bytes,
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
        TerminalStreamMessage::Ownership {
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
        | TerminalStreamMessage::Graphics {
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
        if *has_baseline
            && !*full
            && let Some(previous) = *last_seq
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
    if let TerminalStreamMessage::Graphics { bytes, .. } = message
        && BASE64.decode(bytes.as_bytes()).is_err()
    {
        return Err(("terminal_frame_invalid", "Invalid terminal graphics"));
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
    let config =
        HerdrCliConfig::from_options(None, None, None).expect("failed to load Herdr configuration");
    let inspector = Arc::new(HerdrCliAdapter::new(config).with_server_autostart());
    let startup_inspector = Arc::clone(&inspector);
    let service = CockpitService::new(CockpitMode::Normal, inspector);

    tauri::Builder::default()
        .manage(service)
        .manage(StreamRegistry::new())
        .setup(move |_app| {
            tauri::async_runtime::spawn(async move {
                if let Err(error) = startup_inspector.inspect().await {
                    eprintln!("failed to initialize Herdr: {error}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cockpit_status,
            cockpit_sessions,
            cockpit_session_snapshot,
            cockpit_focus,
            cockpit_mutate,
            cockpit_session_subscribe,
            cockpit_terminal_open,
            cockpit_terminal_command,
            cockpit_stream_cancel
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cockpit Tauri application");
}
