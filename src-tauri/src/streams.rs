use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use cockpit_core::{CockpitService, TerminalSession};
use cockpit_host::{
    BrowserRuntime,
    transport::{
        browser_relay::{self, BrowserViewSubscribeResponse},
        error::OperationError,
        limits,
        session_stream::{self, Incoming, SessionSink, SessionStream},
        terminal,
    },
};
use cockpit_protocol::{
    v1::{
        ErrorResponse, SessionStreamMessage, TerminalCommand, TerminalOpenRequest,
        TerminalStreamMessage,
    },
    widget::{WIDGET_MAX_SNAPSHOT_BYTES, WidgetEvent, WidgetWindowReport},
};
use tauri::{State, ipc::Channel};
use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

const MAX_STREAMS: usize = 256;
const RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

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
    },
    Widget {
        control: Arc<StreamControl>,
        window_id: String,
    },
}

/// Process-local bounded registry for live native streams.
#[derive(Clone)]
pub struct StreamRegistry {
    entries: Arc<Mutex<HashMap<String, StreamEntry>>>,
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn allocate(&self, entry: StreamEntry) -> Result<String, ErrorResponse> {
        let mut entries = self.entries.lock().expect("stream registry lock poisoned");
        if entries.len() >= MAX_STREAMS {
            return Err(OperationError::rejected(
                "stream_limit",
                "The maximum number of active streams has been reached",
            )
            .into());
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

    fn widget_window(&self, stream_id: &str) -> Option<String> {
        let entries = self.entries.lock().expect("stream registry lock poisoned");
        match entries.get(stream_id) {
            Some(StreamEntry::Widget { window_id, .. }) => Some(window_id.clone()),
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
                | StreamEntry::BrowserView { control }
                | StreamEntry::Widget { control, .. } => {
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
                | StreamEntry::BrowserView { control }
                | StreamEntry::Widget { control, .. } => control.cancel().await,
            }
        }
    }
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

struct ChannelSink {
    channel: Channel<SessionStreamMessage>,
    control: Arc<StreamControl>,
}

impl SessionSink for ChannelSink {
    async fn send(&mut self, message: SessionStreamMessage) -> bool {
        self.channel.send(message).is_ok()
    }

    async fn incoming(&mut self) -> Incoming {
        std::future::pending().await
    }

    fn cancelled(&self) -> bool {
        self.control.cancelled.load(Ordering::Acquire)
    }
}

#[tauri::command]
pub async fn cockpit_session_subscribe(
    session_id: String,
    channel: Channel<SessionStreamMessage>,
    service: State<'_, CockpitService>,
    registry: State<'_, StreamRegistry>,
) -> Result<String, ErrorResponse> {
    let (initial, subscription) = session_stream::open_native(service.inner(), &session_id)
        .await
        .map_err(ErrorResponse::from)?;
    let control = StreamControl::new(None);
    let stream_id = registry.allocate(StreamEntry::Session {
        control: Arc::clone(&control),
    })?;
    let task_registry = registry.inner().clone();
    let returned_stream_id = stream_id.clone();
    let task_service = service.inner().clone();
    let mut sink = ChannelSink {
        channel,
        control: Arc::clone(&control),
    };
    let (stream, first) = SessionStream::native(session_id, initial);
    let task = tokio::spawn(async move {
        session_stream::pump(task_service, stream, first, Some(subscription), &mut sink).await;
        task_registry.complete(&stream_id);
    });
    control.set_abort(task.abort_handle());
    Ok(returned_stream_id)
}

#[tauri::command]
pub async fn cockpit_browser_view_subscribe(
    view_id: String,
    stream_epoch: u64,
    runtime: State<'_, Arc<BrowserRuntime>>,
    registry: State<'_, StreamRegistry>,
) -> Result<BrowserViewSubscribeResponse, ErrorResponse> {
    let prepared = browser_relay::prepare(runtime.inner(), &view_id, stream_epoch)
        .await
        .map_err(ErrorResponse::from)?;
    let endpoint = prepared.endpoint().to_owned();
    let grant = prepared.frontend_grant();
    let control = StreamControl::new_browser();
    let stream_id = match registry.allocate(StreamEntry::BrowserView {
        control: Arc::clone(&control),
    }) {
        Ok(stream_id) => stream_id,
        Err(error) => {
            prepared.abandon().await;
            return Err(error);
        }
    };
    let task_registry = registry.inner().clone();
    let task_stream_id = stream_id.clone();
    let task_control = Arc::clone(&control);
    let notify = task_control
        .cancel_notify
        .clone()
        .expect("browser stream cancellation notification");
    let task = tokio::spawn(async move {
        let done = prepared.run(&task_control.cancelled, &notify).await;
        task_registry.complete(&task_stream_id);
        done.release().await;
    });
    control.set_abort(task.abort_handle());
    Ok(BrowserViewSubscribeResponse {
        stream_id,
        endpoint,
        grant,
    })
}

#[tauri::command]
pub async fn cockpit_terminal_open(
    request: TerminalOpenRequest,
    channel: Channel<TerminalStreamMessage>,
    service: State<'_, CockpitService>,
    registry: State<'_, StreamRegistry>,
) -> Result<String, ErrorResponse> {
    let terminal = service
        .open_terminal(&request)
        .await
        .map_err(|error| ErrorResponse::from(OperationError::from(error)))?;
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
    let task_control = Arc::clone(&control);
    let task = tokio::spawn(async move {
        let mut messages = messages;
        let mut last_seq = None;
        let mut has_baseline = false;
        let mut terminal_end = false;
        while !task_control.cancelled.load(Ordering::Acquire) {
            let Some(message) = messages.recv().await else {
                if !terminal_end {
                    let _ = channel.send(terminal::terminal_error(
                        &request,
                        &task_stream_id,
                        "terminal_disconnected",
                        "Terminal stream disconnected",
                    ));
                }
                break;
            };
            if let Err((code, message_text)) = terminal::validate_terminal_message(
                &message,
                &request,
                &herdr_stream_id,
                &mut last_seq,
                &mut has_baseline,
            ) {
                let _ = channel.send(terminal::terminal_error(
                    &request,
                    &task_stream_id,
                    code,
                    message_text,
                ));
                break;
            }
            terminal_end = terminal::ended(&message);
            if channel
                .send(terminal::localize_terminal_message(
                    message,
                    &task_stream_id,
                ))
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
pub async fn cockpit_terminal_command(
    stream_id: String,
    command: TerminalCommand,
    registry: State<'_, StreamRegistry>,
) -> Result<(), ErrorResponse> {
    let command = terminal::command(command).map_err(ErrorResponse::from)?;
    let Some(commands) = registry.terminal_commands(&stream_id) else {
        return Err(OperationError::rejected(
            "stream_not_found",
            "The terminal stream is not active",
        )
        .into());
    };
    commands.send(command).await.map_err(|_| {
        OperationError::rejected("stream_closed", "The terminal stream is closed").into()
    })
}

#[tauri::command]
pub async fn cockpit_stream_cancel(
    stream_id: String,
    registry: State<'_, StreamRegistry>,
) -> Result<(), ErrorResponse> {
    registry.cancel(&stream_id).await;
    Ok(())
}

#[tauri::command]
pub async fn cockpit_widget_report(
    stream_id: String,
    report: WidgetWindowReport,
    runtime: State<'_, Arc<BrowserRuntime>>,
    registry: State<'_, StreamRegistry>,
) -> Result<(), ErrorResponse> {
    limits::widget_request_size(&report).map_err(ErrorResponse::from)?;
    let window_id = registry.widget_window(&stream_id).ok_or_else(|| {
        ErrorResponse::from(OperationError::rejected(
            "widget_usage",
            "Widget subscription is closed",
        ))
    })?;
    runtime
        .widget_report(&window_id, report)
        .await
        .map_err(|error| ErrorResponse::from(OperationError::from(error)))
}

#[tauri::command]
pub async fn cockpit_widget_subscribe(
    channel: Channel<tauri::ipc::Response>,
    error_channel: Channel<ErrorResponse>,
    runtime: State<'_, Arc<BrowserRuntime>>,
    registry: State<'_, StreamRegistry>,
) -> Result<String, ErrorResponse> {
    let mut stream = runtime
        .widget_events()
        .await
        .map_err(|error| ErrorResponse::from(OperationError::from(error)))?;
    let control = StreamControl::new(None);
    let stream_id = registry.allocate(StreamEntry::Widget {
        control: Arc::clone(&control),
        window_id: stream.window_id.clone(),
    })?;
    let returned_id = stream_id.clone();
    let task_registry = registry.inner().clone();
    let task_control = Arc::clone(&control);
    let task = tokio::spawn(async move {
        let send = |event: &WidgetEvent| {
            let Ok(json) = serde_json::to_string(event) else {
                return false;
            };
            json.len() <= WIDGET_MAX_SNAPSHOT_BYTES
                && channel.send(tauri::ipc::Response::new(json)).is_ok()
        };
        if send(&stream.snapshot) {
            loop {
                if task_control.cancelled.load(Ordering::Acquire) {
                    break;
                }
                match stream.events.recv().await {
                    Ok(event) => {
                        if !send(&event) {
                            let _ = error_channel.send(
                                OperationError::rejected(
                                    "widget_stream_closed",
                                    "Widget event delivery failed",
                                )
                                .into(),
                            );
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = error_channel.send(
                            OperationError::rejected(
                                "widget_stream_closed",
                                "Widget stream requires a fresh snapshot",
                            )
                            .into(),
                        );
                        break;
                    }
                }
            }
        } else {
            let _ = error_channel.send(
                OperationError::rejected("widget_stream_closed", "Widget snapshot delivery failed")
                    .into(),
            );
        }
        drop(stream);
        task_registry.complete(&stream_id);
    });
    control.set_abort(task.abort_handle());
    Ok(returned_id)
}
