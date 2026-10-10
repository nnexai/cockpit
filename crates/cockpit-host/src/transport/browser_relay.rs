use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use cockpit_protocol::browser_view::{
    BrowserViewEvent, BrowserViewEventMetadata, BrowserViewSnapshot,
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Notify, broadcast},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::server::{Request as WsRequest, Response as WsResponse},
        protocol::WebSocketConfig,
    },
};
use uuid::Uuid;

use super::{
    browser_frame::{FrameConnection, decode_frame},
    error::OperationError,
};
use crate::browser_runtime::BrowserRuntime;

pub const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
pub const PEER_TIMEOUT: Duration = Duration::from_secs(1);
pub const IO_TIMEOUT: Duration = Duration::from_millis(250);
pub const MAX_MESSAGE_BYTES: usize = 1024;

type BrowserSocket = WebSocketStream<TcpStream>;
type RelayResult = Result<(), (&'static str, String)>;

#[derive(Clone, Debug, Serialize)]
pub struct BrowserViewSubscribeResponse {
    pub stream_id: String,
    pub endpoint: String,
    pub grant: String,
}

pub struct PreparedBrowserRelay {
    relay: BrowserRelay,
    runtime: Arc<BrowserRuntime>,
    view_id: String,
}

struct BrowserRelay {
    listener: TcpListener,
    socket_host: String,
    endpoint: String,
    helper_endpoint: String,
    helper_grant: String,
    frontend_grant: String,
    snapshot: BrowserViewSnapshot,
    stream_epoch: u64,
    events: broadcast::Receiver<BrowserViewEvent>,
}

pub struct FinishedBrowserRelay {
    runtime: Arc<BrowserRuntime>,
    view_id: String,
}

pub async fn prepare(
    runtime: &Arc<BrowserRuntime>,
    view_id: &str,
    stream_epoch: u64,
) -> Result<PreparedBrowserRelay, OperationError> {
    let subscription = runtime
        .browser_view_native_subscribe(view_id, stream_epoch)
        .await?;
    let snapshot = subscription.snapshot;
    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) => {
            runtime
                .browser_view_native_release(&snapshot.identity.view_id)
                .await;
            return Err(OperationError::rejected(
                "browser_socket_unavailable",
                format!("Could not bind browser stream socket: {error}"),
            ));
        }
    };
    let local_addr = match listener.local_addr() {
        Ok(address) => address,
        Err(error) => {
            runtime
                .browser_view_native_release(&snapshot.identity.view_id)
                .await;
            return Err(OperationError::rejected(
                "browser_socket_unavailable",
                format!("Could not read browser stream socket address: {error}"),
            ));
        }
    };
    Ok(PreparedBrowserRelay {
        runtime: Arc::clone(runtime),
        view_id: snapshot.identity.view_id.clone(),
        relay: BrowserRelay {
            listener,
            socket_host: format!("127.0.0.1:{}", local_addr.port()),
            endpoint: format!("ws://127.0.0.1:{}/", local_addr.port()),
            helper_endpoint: subscription.endpoint,
            helper_grant: subscription.grant.grant,
            frontend_grant: Uuid::new_v4().simple().to_string(),
            snapshot,
            stream_epoch,
            events: subscription.events,
        },
    })
}

impl PreparedBrowserRelay {
    pub fn endpoint(&self) -> &str {
        &self.relay.endpoint
    }

    pub fn frontend_grant(&self) -> String {
        self.relay.frontend_grant.clone()
    }

    pub async fn abandon(self) {
        self.runtime
            .browser_view_native_release(&self.view_id)
            .await;
    }

    pub async fn run(self, cancelled: &AtomicBool, cancel_notify: &Notify) -> FinishedBrowserRelay {
        let Self {
            relay,
            runtime,
            view_id,
        } = self;
        // Native callers historically discard the relay result. Completion is
        // returned separately so the registry can complete before release.
        let _ = relay.run_socket(cancelled, cancel_notify).await;
        FinishedBrowserRelay { runtime, view_id }
    }
}

impl FinishedBrowserRelay {
    pub async fn release(self) {
        self.runtime
            .browser_view_native_release(&self.view_id)
            .await;
    }
}

fn valid_browser_socket_origin(origin: &str) -> bool {
    matches!(
        origin,
        "http://localhost:5173"
            | "http://tauri.localhost"
            | "https://tauri.localhost"
            | "tauri://localhost"
    )
}

fn validate_browser_socket_request(request: &WsRequest, expected_host: &str) -> Result<(), ()> {
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .ok_or(())?;
    let origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .ok_or(())?;
    if host != expected_host || !valid_browser_socket_origin(origin) {
        return Err(());
    }
    Ok(())
}

async fn browser_socket_accept(
    listener: &TcpListener,
    expected_host: &str,
) -> Result<BrowserSocket, ()> {
    tokio::time::timeout(PEER_TIMEOUT, async {
        let (stream, _) = listener.accept().await.map_err(|_| ())?;
        stream.set_nodelay(true).map_err(|_| ())?;
        let callback = |request: &WsRequest, response: WsResponse| {
            if validate_browser_socket_request(request, expected_host).is_ok() {
                Ok(response)
            } else {
                Err(WsResponse::builder()
                    .status(403)
                    .body(Some("Forbidden".to_owned()))
                    .expect("valid websocket rejection response"))
            }
        };
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_MESSAGE_BYTES);
        config.max_frame_size = Some(MAX_MESSAGE_BYTES);
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config))
            .await
            .map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
}

async fn browser_socket_send<S>(
    socket: &mut S,
    message: Message,
    cancel_notify: &Notify,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    tokio::select! {
        _ = cancel_notify.notified() => Err(()),
        result = tokio::time::timeout(IO_TIMEOUT, socket.send(message)) => {
            result.map_err(|_| ())?.map_err(|_| ())
        }
    }
}

async fn send_browser_socket_json<S>(
    socket: &mut S,
    message: impl Serialize,
    cancel_notify: &Notify,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let json = serde_json::to_string(&message).map_err(|_| ())?;
    browser_socket_send(socket, Message::Text(json.into()), cancel_notify).await
}

async fn browser_frame_credit(
    frame: &mut FrameConnection,
    kind: &'static str,
    sequence: u64,
) -> Result<(), ()> {
    tokio::time::timeout(IO_TIMEOUT, frame.send_credit(kind, sequence))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

async fn close_socket(socket: &mut BrowserSocket) {
    let _ = tokio::time::timeout(IO_TIMEOUT, socket.close(None)).await;
}

fn socket_closed() -> (&'static str, String) {
    (
        "browser_socket_closed",
        "Browser stream socket closed".to_owned(),
    )
}

fn auth_timeout() -> (&'static str, String) {
    (
        "browser_socket_rejected",
        "Browser stream authentication timed out".to_owned(),
    )
}

fn authorized(
    message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    grant: &str,
) -> bool {
    match message {
        Some(Ok(Message::Text(text))) if text.len() <= MAX_MESSAGE_BYTES => serde_json::from_str::<
            serde_json::Value,
        >(text.as_str())
        .ok()
        .is_some_and(|value| {
            value.as_object().is_some_and(|object| {
                object.len() == 1
                    && object.get("grant").and_then(serde_json::Value::as_str) == Some(grant)
            })
        }),
        _ => false,
    }
}

async fn authenticate(
    listener: &TcpListener,
    socket_host: &str,
    frontend_grant: &str,
    cancel_notify: &Notify,
) -> Result<Option<BrowserSocket>, (&'static str, String)> {
    let auth_deadline = tokio::time::Instant::now() + AUTH_TIMEOUT;
    loop {
        let mut accepted = tokio::select! {
            _ = cancel_notify.notified() => return Ok(None),
            result = tokio::time::timeout_at(auth_deadline, browser_socket_accept(listener, socket_host)) => match result {
                Ok(Ok(socket)) => socket,
                Ok(Err(())) => continue,
                Err(_) => return Err(auth_timeout()),
            },
        };
        let auth = tokio::select! {
            _ = cancel_notify.notified() => {
                close_socket(&mut accepted).await;
                return Ok(None);
            }
            result = tokio::time::timeout_at(auth_deadline, tokio::time::timeout(PEER_TIMEOUT, accepted.next())) => match result {
                Ok(Ok(message)) => message,
                Ok(Err(_)) | Err(_) => {
                    close_socket(&mut accepted).await;
                    if tokio::time::Instant::now() >= auth_deadline {
                        return Err(auth_timeout());
                    }
                    continue;
                }
            },
        };
        if authorized(auth, frontend_grant) {
            return Ok(Some(accepted));
        }
        let _ = send_browser_socket_json(&mut accepted, serde_json::json!({
            "kind": "error", "code": "browser_grant_invalid", "message": "Browser stream grant was invalid",
        }), cancel_notify).await;
        close_socket(&mut accepted).await;
    }
}

async fn send_attached(
    socket: &mut BrowserSocket,
    snapshot: BrowserViewSnapshot,
    stream_epoch: u64,
    cancel_notify: &Notify,
) -> RelayResult {
    send_browser_socket_json(socket, serde_json::json!({"kind": "ready"}), cancel_notify)
        .await
        .map_err(|_| socket_closed())?;
    let metadata = BrowserViewEventMetadata {
        view_id: snapshot.identity.view_id.clone(),
        stream_epoch,
        metadata_sequence: snapshot.metadata_sequence,
    };
    send_browser_socket_json(
        socket,
        serde_json::json!({
            "kind": "event", "event": BrowserViewEvent::Attached { metadata, snapshot },
        }),
        cancel_notify,
    )
    .await
    .map_err(|_| socket_closed())
}

impl BrowserRelay {
    async fn run_socket(mut self, cancelled: &AtomicBool, cancel_notify: &Notify) -> RelayResult {
        let Some(mut socket) = authenticate(
            &self.listener,
            &self.socket_host,
            &self.frontend_grant,
            cancel_notify,
        )
        .await?
        else {
            return Ok(());
        };
        let mut frame = tokio::select! {
            _ = cancel_notify.notified() => return Ok(()),
            result = FrameConnection::connect(&self.helper_endpoint, &self.helper_grant) => match result {
                Ok(frame) => frame,
                Err(error) => {
                    let _ = send_browser_socket_json(&mut socket, serde_json::json!({
                        "kind": "error", "code": error.code, "message": error.message,
                    }), cancel_notify).await;
                    return Err(("browser_frame_connection_failed", "Could not connect browser frame stream".to_owned()));
                }
            },
        };
        let target_id = self
            .snapshot
            .displayed_target_id
            .clone()
            .unwrap_or_default();
        send_attached(&mut socket, self.snapshot, self.stream_epoch, cancel_notify).await?;
        let mut outstanding = None;
        let relay_result = relay_loop(
            &mut socket,
            &mut frame,
            &mut self.events,
            target_id,
            self.stream_epoch,
            &mut outstanding,
            cancelled,
            cancel_notify,
        )
        .await;
        if let Some(sequence) = outstanding {
            let _ = tokio::time::timeout(IO_TIMEOUT, frame.send_credit("discard", sequence)).await;
        }
        close_socket(&mut socket).await;
        relay_result
    }
}

async fn forward_event(
    event: BrowserViewEvent,
    target_id: &mut String,
    socket: &mut BrowserSocket,
    cancel_notify: &Notify,
) -> RelayResult {
    if let BrowserViewEvent::Attached { snapshot, .. } = &event {
        if let Some(target) = &snapshot.displayed_target_id {
            target_id.clone_from(target);
        }
    }
    if let BrowserViewEvent::TargetsChanged {
        displayed_target_id: Some(target),
        ..
    } = &event
    {
        target_id.clone_from(target);
    }
    send_browser_socket_json(
        socket,
        serde_json::json!({"kind": "event", "event": event}),
        cancel_notify,
    )
    .await
    .map_err(|_| socket_closed())
}

async fn forward_credit(
    text: &str,
    frame: &mut FrameConnection,
    outstanding: &mut Option<u64>,
) -> RelayResult {
    let invalid = || {
        (
            "browser_credit_invalid",
            "Invalid browser frame credit".to_owned(),
        )
    };
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| invalid())?;
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != 2 {
        return Err(invalid());
    }
    let credit = object.get("type").and_then(serde_json::Value::as_str);
    let sequence = object
        .get("frame_sequence")
        .and_then(serde_json::Value::as_u64);
    match (credit, sequence, *outstanding) {
        (Some("ack"), Some(sequence), Some(current)) if sequence == current => {
            browser_frame_credit(frame, "ack", sequence)
                .await
                .map_err(|_| {
                    (
                        "browser_frame_credit_failed",
                        "Could not forward browser frame credit".to_owned(),
                    )
                })?;
            *outstanding = None;
        }
        (Some("discard"), Some(sequence), Some(current)) if sequence == current => {
            browser_frame_credit(frame, "discard", sequence)
                .await
                .map_err(|_| {
                    (
                        "browser_frame_credit_failed",
                        "Could not forward browser frame credit".to_owned(),
                    )
                })?;
            *outstanding = None;
        }
        (Some("ack" | "discard"), Some(_), _) => {}
        _ => return Err(invalid()),
    }
    Ok(())
}

async fn forward_frame(
    payload: Vec<u8>,
    target_id: &str,
    stream_epoch: u64,
    socket: &mut BrowserSocket,
    frame: &mut FrameConnection,
    outstanding: &mut Option<u64>,
    cancel_notify: &Notify,
) -> Result<bool, (&'static str, String)> {
    let packet = match decode_frame(&payload, target_id, stream_epoch) {
        Ok(packet) => packet,
        Err(error) => {
            let _ = send_browser_socket_json(
                socket,
                serde_json::json!({
                    "kind": "error", "code": error.code, "message": error.message,
                }),
                cancel_notify,
            )
            .await;
            return Ok(false);
        }
    };
    if outstanding.is_some() {
        browser_frame_credit(frame, "discard", packet.descriptor.frame_sequence)
            .await
            .map_err(|_| {
                (
                    "browser_frame_credit_failed",
                    "Could not discard pending browser frame".to_owned(),
                )
            })?;
        return Ok(true);
    }
    let descriptor = packet.descriptor;
    *outstanding = Some(descriptor.frame_sequence);
    send_browser_socket_json(
        socket,
        serde_json::json!({"kind": "frame", "descriptor": descriptor}),
        cancel_notify,
    )
    .await
    .map_err(|_| socket_closed())?;
    browser_socket_send(socket, Message::Binary(packet.jpeg.into()), cancel_notify)
        .await
        .map_err(|_| socket_closed())?;
    Ok(true)
}

async fn relay_loop(
    socket: &mut BrowserSocket,
    frame: &mut FrameConnection,
    events: &mut broadcast::Receiver<BrowserViewEvent>,
    mut target_id: String,
    stream_epoch: u64,
    outstanding: &mut Option<u64>,
    cancelled: &AtomicBool,
    cancel_notify: &Notify,
) -> RelayResult {
    while !cancelled.load(Ordering::Acquire) {
        tokio::select! {
            _ = cancel_notify.notified() => break,
            event = events.recv() => match event {
                Ok(event) => forward_event(event, &mut target_id, socket, cancel_notify).await?,
                Err(_) => {
                    let _ = send_browser_socket_json(socket, serde_json::json!({
                        "kind": "error", "code": "browser_metadata_closed", "message": "Browser metadata stream closed",
                    }), cancel_notify).await;
                    break;
                }
            },
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) if text.len() <= MAX_MESSAGE_BYTES => {
                    forward_credit(text.as_str(), frame, outstanding).await?;
                }
                Some(Ok(Message::Ping(payload))) => {
                    browser_socket_send(socket, Message::Pong(payload), cancel_notify)
                        .await.map_err(|_| socket_closed())?;
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => return Err(("browser_message_invalid", "Invalid browser stream message".to_owned())),
            },
            incoming = frame.recv() => match incoming {
                Ok(Some(payload)) => {
                    if !forward_frame(payload, &target_id, stream_epoch, socket, frame, outstanding, cancel_notify).await? {
                        break;
                    }
                }
                Ok(None) | Err(_) => break,
            }
        }
    }
    Ok(())
}
