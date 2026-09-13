use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, LazyLock},
};

use axum::{
    Extension, Json, Router,
    extract::{
        DefaultBodyLimit, Path as AxumPath,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cockpit_core::{CockpitService, InspectionError};
use cockpit_protocol::browser_view::{
    BROWSER_VIEW_FRAME_MAX_HEIGHT, BROWSER_VIEW_FRAME_MAX_JPEG_BYTES, BROWSER_VIEW_FRAME_MAX_WIDTH,
    BROWSER_VIEW_FRAME_V2_HEADER_BYTES, BROWSER_VIEW_FRAME_V2_MAGIC, BROWSER_VIEW_FRAME_V2_VERSION,
    BrowserDraftRecoveryRequest, BrowserViewCommandRequest, BrowserViewEvent, BrowserViewEventMetadata,
    BrowserViewFrameDescriptor, BrowserViewFrameGrant, BrowserViewOpenRequest, BrowserViewSnapshot,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{WebSocketStream, tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message as FrameMessage}};

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

const MAX_JSON: usize = 128 * 1024;
const MAX_FRAME: usize =
    BROWSER_VIEW_FRAME_V2_HEADER_BYTES as usize + BROWSER_VIEW_FRAME_MAX_JPEG_BYTES as usize;

#[derive(Clone, Debug, Serialize)]
pub struct BrowserViewOpenResponse {
    pub snapshot: BrowserViewSnapshot,
    pub first_frame: BrowserViewFrameDescriptor,
    /// Always a gateway route. The private helper endpoint never crosses this seam.
    pub frame_endpoint: String,
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

pub fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/browser/view/open", post(open))
        .route("/api/v1/browser/drafts/recovery", post(draft_recovery))
        .route("/api/v1/browser/view/command", post(command).layer(DefaultBodyLimit::max(6 * 1024 * 1024)))
        .route("/api/v1/browser/view/events/{view_id}", get(events))
        .route("/api/v1/browser/view/frame/{view_id}", get(frame))
        .layer(DefaultBodyLimit::max(MAX_JSON))
}

async fn draft_recovery(
    Extension(runtime): Extension<Option<Arc<crate::browser_runtime::BrowserRuntime>>>,
    Json(request): Json<BrowserDraftRecoveryRequest>,
) -> Response {
    let Some(runtime) = runtime else {
        return error_response("browser_runtime_unavailable", "Browser runtime is not configured");
    };
    match runtime.browser_draft_recovery(request).await {
        Ok(outcome) => Json(outcome).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn open(
    Extension(runtime): Extension<Option<Arc<crate::browser_runtime::BrowserRuntime>>>,
    Json(request): Json<BrowserViewOpenRequest>,
) -> Response {
    if let Err(message) = request.validate() {
        return error_response("invalid_browser_view_open", message);
    }
    let Some(runtime) = runtime else {
        return error_response(
            "browser_runtime_unavailable",
            "Browser runtime is not configured",
        );
    };
    match runtime.open_browser_view(request).await {
        Ok(opened) => {
            let view_id = opened.snapshot.identity.view_id.clone();
            Json(BrowserViewOpenResponse {
                snapshot: opened.snapshot,
                first_frame: opened.first_frame,
                frame_endpoint: format!("/api/v1/browser/view/frame/{view_id}"),
            })
            .into_response()
        }
        Err(error) => inspection_error(error),
    }
}

async fn command(
    Extension(runtime): Extension<Option<Arc<crate::browser_runtime::BrowserRuntime>>>,
    Json(request): Json<BrowserViewCommandRequest>,
) -> Response {
    if let Err(error) = crate::browser_runtime::validate_browser_view_command(&request) {
        return error_response(&error.code, &error.message);
    }
    let Some(runtime) = runtime else {
        return error_response(
            "browser_runtime_unavailable",
            "Browser runtime is not configured",
        );
    };
    match runtime.browser_view_command(request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn events(
    ws: WebSocketUpgrade,
    Extension(runtime): Extension<Option<Arc<crate::browser_runtime::BrowserRuntime>>>,
    AxumPath(view_id): AxumPath<String>,
) -> Response {
    if !valid_id(&view_id) {
        return error_response("invalid_browser_view_id", "Invalid browser view id");
    }
    let Some(runtime) = runtime else {
        return error_response(
            "browser_runtime_unavailable",
            "Browser runtime is not configured",
        );
    };
    let events = match runtime.browser_view_events(&view_id).await {
        Ok(events) => events,
        Err(error) => return inspection_error(error),
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
        return error_response("invalid_browser_view_id", "Invalid browser view id");
    }
    let Some(runtime) = runtime else {
        return error_response(
            "browser_runtime_unavailable",
            "Browser runtime is not configured",
        );
    };
    ws.max_message_size(MAX_JSON)
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
    retain_view_connection(&runtime, &view_id).await;
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
    let Ok(Some(first)) = tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv()).await else {
        return;
    };
    let grant = match first {
        Ok(Message::Text(text)) if text.len() <= MAX_JSON => {
            match serde_json::from_str::<FrameGrantMessage>(&text) {
                Ok(message) if message.grant.view_id == view_id => message.grant,
                _ => {
                    close_ws(&mut socket, 1008, "invalid browser frame grant").await;
                    return;
                }
            }
        }
        _ => {
            close_ws(&mut socket, 1008, "frame grant required").await;
            return;
        }
    };
    let endpoint = match runtime.browser_view_frame_endpoint(&grant).await {
        Ok(endpoint) => endpoint,
        Err(error) => {
            close_ws(&mut socket, 1008, &error.message).await;
            return;
        }
    };
    // The runtime has validated the grant at this point. Hold a connection
    // lease while the private helper lane is established so a transport
    // failure cleans up this stream's own view without affecting peers.
    retain_view_connection(&runtime, &view_id).await;
    let mut helper = match FrameConnection::connect(&endpoint, &grant.grant).await {
        Ok(connection) => connection,
        Err(_) => {
            close_ws(&mut socket, 1011, "browser frame transport unavailable").await;
            release_view_connection(&runtime, &view_id).await;
            return;
        }
    };
    let mut target_id = String::new();
    let mut metadata_events = match runtime.browser_view_events(&view_id).await {
        Ok(events) => {
            target_id = events.snapshot.displayed_target_id.unwrap_or_default();
            events.events
        }
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
                    if text.len() > MAX_JSON { close_ws(&mut socket, 1009, "frame credit message too large").await; break; }
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
fn error_response(code: &str, message: &str) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(cockpit_protocol::v1::ErrorResponse {
            code: code.to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}
fn inspection_error(error: InspectionError) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(cockpit_protocol::v1::ErrorResponse {
            code: error.code.to_owned(),
            message: error.message.to_owned(),
        }),
    )
        .into_response()
}

#[derive(Debug)]
pub struct FramePacket {
    pub descriptor: BrowserViewFrameDescriptor,
    pub jpeg: Vec<u8>,
}

pub fn decode_frame(
    payload: &[u8],
    target_id: &str,
    expected_epoch: u64,
) -> Result<FramePacket, InspectionError> {
    let header_len = BROWSER_VIEW_FRAME_V2_HEADER_BYTES as usize;
    if payload.len() < header_len {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame header is truncated",
        ));
    }
    let read_u16 =
        |offset: usize| u16::from_be_bytes(payload[offset..offset + 2].try_into().unwrap());
    let read_u32 =
        |offset: usize| u32::from_be_bytes(payload[offset..offset + 4].try_into().unwrap());
    let read_u64 =
        |offset: usize| u64::from_be_bytes(payload[offset..offset + 8].try_into().unwrap());
    if read_u32(0) != BROWSER_VIEW_FRAME_V2_MAGIC
        || read_u16(4) != BROWSER_VIEW_FRAME_V2_VERSION
        || read_u16(6) != BROWSER_VIEW_FRAME_V2_HEADER_BYTES
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame envelope is not IBFV v2",
        ));
    }

    let epoch = read_u64(8);
    let frame_sequence = read_u64(16);
    let document_generation = read_u64(24);
    let viewport_revision = read_u64(32);
    let jpeg_len = read_u32(80) as usize;
    if epoch != expected_epoch
        || frame_sequence == 0
        || document_generation == 0
        || viewport_revision == 0
        || jpeg_len == 0
        || jpeg_len > BROWSER_VIEW_FRAME_MAX_JPEG_BYTES as usize
        || payload.len() != header_len + jpeg_len
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame identity, sequence, generation, or length is invalid",
        ));
    }
    if read_u32(84) != 0 || payload[88..header_len].iter().any(|byte| *byte != 0) {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame flags or reserved bytes are non-zero",
        ));
    }
    if target_id.is_empty() {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame target identity is unavailable",
        ));
    }

    let viewport_css_width = f32::from_bits(read_u32(48));
    let viewport_css_height = f32::from_bits(read_u32(52));
    let viewport_offset_x = f32::from_bits(read_u32(56));
    let viewport_offset_y = f32::from_bits(read_u32(60));
    let scroll_x = f32::from_bits(read_u32(64));
    let scroll_y = f32::from_bits(read_u32(68));
    if !viewport_css_width.is_finite()
        || !viewport_css_height.is_finite()
        || !viewport_offset_x.is_finite()
        || !viewport_offset_y.is_finite()
        || !scroll_x.is_finite()
        || !scroll_y.is_finite()
        || viewport_css_width <= 0.0
        || viewport_css_height <= 0.0
        || viewport_css_width > BROWSER_VIEW_FRAME_MAX_WIDTH as f32
        || viewport_css_height > BROWSER_VIEW_FRAME_MAX_HEIGHT as f32
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame geometry is non-finite or outside bounds",
        ));
    }

    let image_width = read_u32(40);
    let image_height = read_u32(44);
    let jpeg = &payload[header_len..];
    if jpeg.len() < 2
        || jpeg[0] != 0xff
        || jpeg[1] != 0xd8
        || jpeg[jpeg.len() - 2] != 0xff
        || jpeg[jpeg.len() - 1] != 0xd9
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame payload is not a complete JPEG",
        ));
    }
    let descriptor = BrowserViewFrameDescriptor {
        target_id: target_id.to_owned(),
        stream_epoch: epoch,
        frame_sequence,
        document_generation,
        viewport_revision,
        image_width,
        image_height,
        viewport_css_width: viewport_css_width as f64,
        viewport_css_height: viewport_css_height as f64,
        viewport_offset_x: viewport_offset_x as f64,
        viewport_offset_y: viewport_offset_y as f64,
        scroll_x: scroll_x as f64,
        scroll_y: scroll_y as f64,
        capture_timestamp_micros: read_u64(72),
        jpeg_length: jpeg_len as u32,
    };
    descriptor
        .validate()
        .map_err(|message| InspectionError::new("browser_frame_invalid", message))?;
    Ok(FramePacket {
        descriptor,
        jpeg: jpeg.to_vec(),
    })
}

pub struct FrameConnection {
    stream: WebSocketStream<TcpStream>,
}
impl FrameConnection {
    pub async fn send_credit(&mut self, kind: &str, frame_sequence: u64) -> Result<(), InspectionError> {
        self.stream.send(FrameMessage::Text(
            serde_json::json!({"type": kind, "frame_sequence": frame_sequence}).to_string().into()
        )).await.map_err(|_| frame_error("Could not return browser frame credit"))
    }

    pub async fn connect(endpoint: &str, grant: &str) -> Result<Self, InspectionError> {
        let authority_path = endpoint.strip_prefix("ws://")
            .ok_or_else(|| frame_error("Browser frame endpoint is not loopback WebSocket"))?;
        let authority = authority_path.split('/').next().unwrap_or_default();
        let address: SocketAddr = authority.parse()
            .map_err(|_| frame_error("Browser frame endpoint is invalid"))?;
        if !address.ip().is_loopback() {
            return Err(frame_error("Browser frame endpoint is not loopback"));
        }
        let mut request = endpoint.into_client_request()
            .map_err(|_| frame_error("Browser frame endpoint is invalid"))?;
        request.headers_mut().insert("Origin", format!("http://{authority}").parse()
            .map_err(|_| frame_error("Browser frame origin is invalid"))?);
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let connection = async {
            let tcp = TcpStream::connect(address).await
                .map_err(|_| frame_error("Could not connect browser frame endpoint"))?;
            let (mut stream, _) = tokio_tungstenite::client_async_with_config(request, tcp, Some(config)).await
                .map_err(|_| frame_error("Browser frame handshake failed"))?;
            stream.send(FrameMessage::Text(serde_json::json!({"grant": grant}).to_string().into())).await
                .map_err(|_| frame_error("Could not authorize browser frame endpoint"))?;
            Ok(Self { stream })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), connection).await
            .map_err(|_| frame_error("Browser frame connection timed out"))?
    }

    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, InspectionError> {
        loop {
            match self.stream.next().await {
                Some(Ok(FrameMessage::Binary(payload))) => return Ok(Some(payload.to_vec())),
                None | Some(Ok(FrameMessage::Close(_))) => return Ok(None),
                Some(Ok(FrameMessage::Ping(_))) => {
                    self.stream.flush().await.map_err(|_| frame_error("Browser frame heartbeat failed"))?;
                }
                Some(Ok(FrameMessage::Pong(_))) => {},
                _ => return Err(frame_error("Invalid browser frame message")),
            }
        }
    }
}
fn frame_error(message: &str) -> InspectionError {
    InspectionError::new("browser_frame_unavailable", message)
}
