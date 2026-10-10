use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use axum::{
    Extension, Router,
    extract::{
        DefaultBodyLimit, Path as AxumPath,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use cockpit_core::CockpitService;
use cockpit_protocol::browser_view::{
    BrowserViewEvent, BrowserViewEventMetadata, BrowserViewFrameGrant, BrowserViewSnapshot,
};
use serde::{Deserialize, Serialize};

use crate::transport::{
    browser_frame::{FrameConnection, MAX_FRAME, decode_frame},
    error::{OperationError, StatusPolicy},
    guard::{Missing, require},
    limits::BROWSER_VIEW_JSON_BYTES,
};

/// Number of accepted gateway streams currently holding each managed view. A
/// view is detached only when its final event or frame stream closes.
///
/// The runtime address is part of the key so an independently restarted
/// runtime cannot release another runtime's view if a view id is reused.
static VIEW_CONNECTIONS: LazyLock<tokio::sync::Mutex<HashMap<(usize, String), usize>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

fn view_connections() -> &'static tokio::sync::Mutex<HashMap<(usize, String), usize>> {
    &VIEW_CONNECTIONS
}

fn view_connection_key(
    runtime: &Arc<crate::browser_runtime::BrowserRuntime>,
    view_id: &str,
) -> (usize, String) {
    (Arc::as_ptr(runtime) as usize, view_id.to_owned())
}

async fn retain_view_connection(
    runtime: &Arc<crate::browser_runtime::BrowserRuntime>,
    view_id: &str,
) {
    let mut connections = view_connections().lock().await;
    *connections
        .entry(view_connection_key(runtime, view_id))
        .or_insert(0) += 1;
}

async fn release_view_connection(
    runtime: &Arc<crate::browser_runtime::BrowserRuntime>,
    view_id: &str,
) {
    let mut connections = view_connections().lock().await;
    let key = view_connection_key(runtime, view_id);
    let Some(count) = connections.get_mut(&key) else {
        return;
    };
    if *count > 1 {
        *count -= 1;
        return;
    }
    connections.remove(&key);
    // Keep the registry locked through detach so a newly opened stream cannot
    // race this final release and then be destroyed by the old owner.
    let _ = runtime.browser_view_detach(view_id).await;
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameGrantMessage {
    grant: BrowserViewFrameGrant,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameCreditMessage {
    #[serde(rename = "type")]
    kind: String,
    frame_sequence: u64,
}

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/browser/view/events/{view_id}", get(events))
        .route("/api/v1/browser/view/frame/{view_id}", get(frame))
        .layer(DefaultBodyLimit::max(BROWSER_VIEW_JSON_BYTES))
}

async fn events(
    ws: WebSocketUpgrade,
    Extension(runtime): Extension<Option<Arc<crate::browser_runtime::BrowserRuntime>>>,
    AxumPath(view_id): AxumPath<String>,
) -> Response {
    if !valid_id(&view_id) {
        return OperationError::rejected("invalid_browser_view_id", "Invalid browser view id")
            .into_response(StatusPolicy::BadRequest);
    }
    let runtime = match require(runtime, Missing::Browser) {
        Ok(runtime) => runtime,
        Err(error) => return error.into_response(StatusPolicy::BadRequest),
    };
    // Lease the view before taking the snapshot. A final release from an older
    // stream must not remove it between lookup and WebSocket upgrade.
    retain_view_connection(&runtime, &view_id).await;
    let events = match runtime.browser_view_events(&view_id).await {
        Ok(events) => events,
        Err(error) => {
            release_view_connection(&runtime, &view_id).await;
            return OperationError::from(error).into_response(StatusPolicy::BadRequest);
        }
    };
    ws.on_upgrade(move |socket| {
        run_events(socket, runtime, events.snapshot, events.events, view_id)
    })
    .into_response()
}

async fn frame(
    ws: WebSocketUpgrade,
    Extension(runtime): Extension<Option<Arc<crate::browser_runtime::BrowserRuntime>>>,
    AxumPath(view_id): AxumPath<String>,
) -> Response {
    if !valid_id(&view_id) {
        return OperationError::rejected("invalid_browser_view_id", "Invalid browser view id")
            .into_response(StatusPolicy::BadRequest);
    }
    let runtime = match require(runtime, Missing::Browser) {
        Ok(runtime) => runtime,
        Err(error) => return error.into_response(StatusPolicy::BadRequest),
    };
    // Hold the lease before validating the grant so event/frame upgrade order
    // cannot let the final older stream detach this view.
    retain_view_connection(&runtime, &view_id).await;
    ws.max_message_size(BROWSER_VIEW_JSON_BYTES)
        .on_upgrade(move |socket| run_frame(socket, runtime, view_id))
        .into_response()
}

async fn run_events(
    mut socket: WebSocket,
    runtime: Arc<crate::browser_runtime::BrowserRuntime>,
    snapshot: BrowserViewSnapshot,
    mut events: tokio::sync::broadcast::Receiver<BrowserViewEvent>,
    view_id: String,
) {
    let metadata = BrowserViewEventMetadata {
        view_id: snapshot.identity.view_id.clone(),
        stream_epoch: snapshot.identity.stream_epoch,
        metadata_sequence: snapshot.metadata_sequence,
    };
    if !send_json(
        &mut socket,
        &BrowserViewEvent::Attached { metadata, snapshot },
    )
    .await
    {
        release_view_connection(&runtime, &view_id).await;
        return;
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Ping(payload))) => if socket.send(Message::Pong(payload)).await.is_err() { break },
                Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) | Some(Ok(Message::Pong(_))) => {}
            },
            next = events.recv() => match next {
                Ok(event) => if !send_json(&mut socket, &event).await { break },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => { close_ws(&mut socket, 1013, "browser metadata stream lagged").await; break; }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    release_view_connection(&runtime, &view_id).await;
}

async fn run_frame(
    mut socket: WebSocket,
    runtime: Arc<crate::browser_runtime::BrowserRuntime>,
    view_id: String,
) {
    let Ok(Some(first)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv()).await
    else {
        release_view_connection(&runtime, &view_id).await;
        return;
    };
    let grant = match first {
        Ok(Message::Text(text)) if text.len() <= BROWSER_VIEW_JSON_BYTES => {
            match serde_json::from_str::<FrameGrantMessage>(&text) {
                Ok(message) if message.grant.view_id == view_id => message.grant,
                _ => {
                    close_ws(&mut socket, 1008, "invalid browser frame grant").await;
                    release_view_connection(&runtime, &view_id).await;
                    return;
                }
            }
        }
        _ => {
            close_ws(&mut socket, 1008, "frame grant required").await;
            release_view_connection(&runtime, &view_id).await;
            return;
        }
    };
    let endpoint = match runtime.browser_view_frame_endpoint(&grant).await {
        Ok(endpoint) => endpoint,
        Err(error) => {
            close_ws(&mut socket, 1008, &error.message).await;
            release_view_connection(&runtime, &view_id).await;
            return;
        }
    };
    let mut helper = match FrameConnection::connect(&endpoint, &grant.grant).await {
        Ok(connection) => connection,
        Err(_) => {
            close_ws(&mut socket, 1011, "browser frame transport unavailable").await;
            release_view_connection(&runtime, &view_id).await;
            return;
        }
    };
    let (mut metadata_events, mut target_id) = match runtime.browser_view_events(&view_id).await {
        Ok(events) => (
            events.events,
            events.snapshot.displayed_target_id.unwrap_or_default(),
        ),
        Err(_) => {
            close_ws(&mut socket, 1011, "browser metadata unavailable").await;
            release_view_connection(&runtime, &view_id).await;
            return;
        }
    };
    let mut outstanding: Option<u64> = None;
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Ping(payload))) => if socket.send(Message::Pong(payload)).await.is_err() { break },
                Some(Ok(Message::Text(text))) => {
                    if text.len() > BROWSER_VIEW_JSON_BYTES { close_ws(&mut socket, 1009, "frame credit message too large").await; break; }
                    let Ok(credit) = serde_json::from_str::<FrameCreditMessage>(&text) else { close_ws(&mut socket, 1003, "invalid frame credit message").await; break; };
                    if !matches!(credit.kind.as_str(), "ack" | "discard") || outstanding != Some(credit.frame_sequence) { close_ws(&mut socket, 1008, "invalid frame credit").await; break; }
                    if helper.send_credit(&credit.kind, credit.frame_sequence).await.is_err() { break; }
                    outstanding = None;
                }
                Some(Ok(Message::Binary(_))) | Some(Ok(Message::Pong(_))) => {}
            },
            metadata = metadata_events.recv() => match metadata {
                Ok(BrowserViewEvent::Attached { snapshot, .. }) => {
                    target_id = snapshot.displayed_target_id.unwrap_or_default();
                }
                Ok(BrowserViewEvent::TargetsChanged { displayed_target_id, .. }) => {
                    target_id = displayed_target_id.unwrap_or_default();
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    close_ws(&mut socket, 1013, "browser metadata stream lagged").await;
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            incoming = helper.recv() => match incoming {
                Ok(Some(payload)) => {
                    if payload.len() > MAX_FRAME { close_ws(&mut socket, 1009, "browser frame too large").await; break; }
                    let Ok(packet) = decode_frame(&payload, &target_id, grant.stream_epoch) else { close_ws(&mut socket, 1003, "invalid browser frame").await; break; };
                    if outstanding.is_some() {
                        if helper.send_credit("discard", packet.descriptor.frame_sequence).await.is_err() { break; }
                        continue;
                    }
                    let sequence = packet.descriptor.frame_sequence;
                    if send_frame(&mut socket, payload).await.is_err() { break; }
                    outstanding = Some(sequence);
                }
                Ok(None) | Err(_) => break,
            }
        }
    }
    if let Some(sequence) = outstanding {
        let _ = helper.send_credit("discard", sequence).await;
    }
    release_view_connection(&runtime, &view_id).await;
}

async fn send_frame(socket: &mut WebSocket, payload: Vec<u8>) -> Result<(), axum::Error> {
    socket.send(Message::Binary(payload.into())).await
}
async fn send_json<T: Serialize>(socket: &mut WebSocket, value: &T) -> bool {
    let Ok(text) = serde_json::to_string(value) else {
        return false;
    };
    socket.send(Message::Text(text.into())).await.is_ok()
}
async fn close_ws(socket: &mut WebSocket, code: u16, reason: &str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.to_owned().into(),
        })))
        .await;
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
