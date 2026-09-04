use std::{
    convert::Infallible,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Path as AxumPath, Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, Request, StatusCode, Uri,
        header::{HOST, ORIGIN},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use cockpit_core::{
    CockpitService, InspectionError, SessionChange, SessionSubscription, TerminalSession,
};
use cockpit_protocol::v1::{
    ErrorResponse, FocusRequest, ResourceMutationRequest, ResourceMutationResponse,
    SessionSnapshotResponse, SessionStreamMessage, TerminalCommand, TerminalMode,
    TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage,
};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
const MAX_MUTATION_REQUEST_BYTES: usize = 64 * 1024;

/// Configuration for the foreground HTTP gateway.
#[derive(Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub static_dir: PathBuf,
    pub service: CockpitService,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("bind address must be loopback: {0}")]
    NonLoopback(SocketAddr),
    #[error("static root `{path}` is missing or unreadable: {source}")]
    StaticRoot {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("static root `{0}` is not a directory")]
    StaticRootNotDirectory(PathBuf),
    #[error("static index `{path}` is missing or unreadable: {source}")]
    StaticIndex {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("HTTP server failed: {0}")]
    Serve(#[source] std::io::Error),
}

/// Reject remote binds before creating a listener.
pub fn validate_bind(bind: SocketAddr) -> Result<(), ServerError> {
    if bind.ip().is_loopback() {
        Ok(())
    } else {
        Err(ServerError::NonLoopback(bind))
    }
}

/// Validate the static root and its entry point before startup binds a socket.
pub fn validate_static_root(root: impl AsRef<Path>) -> Result<PathBuf, ServerError> {
    let root = root.as_ref().to_path_buf();
    let metadata = std::fs::metadata(&root).map_err(|source| ServerError::StaticRoot {
        path: root.clone(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(ServerError::StaticRootNotDirectory(root));
    }
    std::fs::read_dir(&root).map_err(|source| ServerError::StaticRoot {
        path: root.clone(),
        source,
    })?;

    let index = root.join("index.html");
    let index_metadata = std::fs::metadata(&index).map_err(|source| ServerError::StaticIndex {
        path: index.clone(),
        source,
    })?;
    if !index_metadata.is_file() {
        return Err(ServerError::StaticIndex {
            path: index,
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, "not a regular file"),
        });
    }
    let file = std::fs::File::open(&index).map_err(|source| ServerError::StaticIndex {
        path: index.clone(),
        source,
    })?;
    drop(file);
    Ok(root)
}

/// Build the reusable Axum router for the bounded Cockpit API.
///
/// `expected_authority` is the listener's actual loopback address, including
/// its port. Requests must carry that exact numeric authority in `Host`.
pub fn build_router(
    service: CockpitService,
    static_dir: impl AsRef<Path>,
    expected_authority: SocketAddr,
) -> Result<Router, ServerError> {
    validate_bind(expected_authority)?;
    let static_dir = validate_static_root(static_dir)?;
    Ok(build_router_with_validated_root(
        service,
        static_dir,
        expected_authority,
    ))
}

fn build_router_with_validated_root(
    service: CockpitService,
    static_dir: PathBuf,
    expected_authority: SocketAddr,
) -> Router {
    let expected_authority = expected_authority.to_string();
    let expected_origin = format!("http://{expected_authority}");
    let index = static_dir.join("index.html");
    let static_service = ServeDir::new(static_dir)
        .append_index_html_on_directories(true)
        .fallback(tower::service_fn(move |request: Request<Body>| {
            let index = index.clone();
            async move { Ok::<_, Infallible>(static_not_found(request.uri(), index).await) }
        }));

    Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/sessions", get(sessions))
        .route(
            "/api/v1/sessions/{session_id}/snapshot",
            get(session_snapshot),
        )
        .route("/api/v1/sessions/{session_id}/focus", post(focus))
        .route(
            "/api/v1/sessions/{session_id}/mutations",
            post(mutate).layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES)),
        )
        .route("/api/v1/sessions/{session_id}/events", get(session_events))
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/terminal",
            get(terminal_ws),
        )
        // Keep API resolution ahead of the static service. This prevents a static file
        // named api/... from changing the API's 404 contract.
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found))
        .fallback_service(static_service)
        .with_state(service)
        .layer(middleware::from_fn(
            move |request: Request<Body>, next: Next| {
                let allowed = authority_headers_match(
                    request.headers(),
                    &expected_authority,
                    &expected_origin,
                );
                enforce_authority(request, next, allowed)
            },
        ))
}

/// Short alias for callers that prefer the conventional router constructor name.
pub fn router(
    service: CockpitService,
    static_dir: impl AsRef<Path>,
    expected_authority: SocketAddr,
) -> Result<Router, ServerError> {
    build_router(service, static_dir, expected_authority)
}

async fn enforce_authority(request: Request<Body>, next: Next, allowed: bool) -> Response {
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                code: "invalid_request_authority".to_owned(),
                message: "Request authority is not allowed".to_owned(),
            }),
        )
            .into_response();
    }
    next.run(request).await
}

fn authority_headers_match(
    headers: &HeaderMap,
    expected_authority: &str,
    expected_origin: &str,
) -> bool {
    let mut hosts = headers.get_all(HOST).iter();
    let host_matches = hosts
        .next()
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| host == expected_authority);
    if !host_matches || hosts.next().is_some() {
        return false;
    }

    let mut origins = headers.get_all(ORIGIN).iter();
    match origins.next() {
        None => true,
        Some(origin) => {
            origins.next().is_none()
                && origin
                    .to_str()
                    .is_ok_and(|origin| origin == expected_origin)
        }
    }
}

async fn status(State(service): State<CockpitService>) -> impl IntoResponse {
    Json(service.status().await)
}

async fn sessions(State(service): State<CockpitService>) -> Response {
    match service.sessions().await {
        Ok(sessions) => Json(sessions).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn session_snapshot(
    State(service): State<CockpitService>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Invalid session id");
    }
    match service.session_snapshot(&session_id).await {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn focus(
    State(service): State<CockpitService>,
    AxumPath(session_id): AxumPath<String>,
    Json(request): Json<FocusRequest>,
) -> Response {
    if !valid_session_id(&session_id) || !valid_resource_id(&request.target_id) {
        return bad_request("invalid_focus_target", "Invalid focus target");
    }
    match service.focus(&session_id, &request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn mutate(
    State(service): State<CockpitService>,
    AxumPath(session_id): AxumPath<String>,
    Json(request): Json<ResourceMutationRequest>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Invalid session id");
    }
    match service.mutate(&session_id, &request).await {
        Ok(response) => Json::<ResourceMutationResponse>(response).into_response(),
        Err(error) => inspection_error(error),
    }
}
const MAX_TERMINAL_COMMAND_BYTES: usize = 96 * 1024;
#[derive(Debug, Deserialize)]
struct TerminalQuery {
    mode: TerminalMode,
    takeover: bool,
    cols: u16,
    rows: u16,
}

async fn session_events(
    ws: WebSocketUpgrade,
    State(service): State<CockpitService>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Invalid session id");
    }
    ws.on_upgrade(move |socket| run_session_socket(socket, service, session_id))
        .into_response()
}

async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(service): State<CockpitService>,
    AxumPath((session_id, pane_id)): AxumPath<(String, String)>,
    Query(query): Query<TerminalQuery>,
) -> Response {
    if !valid_session_id(&session_id) || !valid_resource_id(&pane_id) {
        return bad_request("invalid_terminal_target", "Invalid terminal target");
    }
    if !valid_dimensions(query.cols, query.rows) {
        return bad_request("invalid_terminal_dimensions", "Invalid terminal dimensions");
    }
    let request = TerminalOpenRequest {
        session_id,
        pane_id,
        mode: query.mode,
        takeover: query.takeover,
        cols: query.cols,
        rows: query.rows,
    };
    let session = match service.open_terminal(&request).await {
        Ok(session) => session,
        Err(error) => return inspection_error(error),
    };
    ws.max_message_size(MAX_TERMINAL_COMMAND_BYTES)
        .on_upgrade(move |socket| run_terminal_socket(socket, session, request))
        .into_response()
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_resource_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_'))
}

fn valid_dimensions(cols: u16, rows: u16) -> bool {
    cols > 0 && rows > 0
}

fn inspection_error(error: InspectionError) -> Response {
    let status = if error.code.starts_with("invalid_") {
        StatusCode::BAD_REQUEST
    } else if error.code == "focus_conflict" || error.code == "terminal_ownership_conflict" {
        StatusCode::CONFLICT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(ErrorResponse {
            code: error.code,
            message: error.message,
        }),
    )
        .into_response()
}

fn bad_request(code: &str, message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            code: code.to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

async fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            code: "not_found".to_owned(),
            message: "API route not found".to_owned(),
        }),
    )
}

async fn static_not_found(uri: &Uri, index: PathBuf) -> Response {
    let encoded_path = uri.path().trim_start_matches('/');
    let path = match percent_decode_str(encoded_path).decode_utf8() {
        Ok(path) if !path.as_bytes().contains(&0) => path,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if path == "api" || path.starts_with("api/") {
        return api_not_found().await.into_response();
    }
    if path.split('/').any(|component| component == "..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if Path::new(path.as_ref()).extension().is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }
    use tower::ServiceExt;
    match ServeFile::new(index)
        .oneshot(Request::new(Body::empty()))
        .await
    {
        Ok(response) => response.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn send_json<T: serde::Serialize>(socket: &mut WebSocket, value: &T) -> bool {
    let Ok(text) = serde_json::to_string(value) else {
        return false;
    };
    socket.send(Message::Text(text.into())).await.is_ok()
}

async fn send_session(
    socket: &mut WebSocket,
    generation: &mut u32,
    sequence: &mut u32,
    message: SessionStreamMessage,
) -> bool {
    if *sequence == 0 {
        return false;
    }
    if !send_json(socket, &message).await {
        return false;
    }
    if *sequence == u32::MAX {
        *generation = match generation.checked_add(1) {
            Some(value) => value,
            None => return false,
        };
        *sequence = 1;
    } else {
        *sequence += 1;
    }
    true
}

fn session_snapshot_message(
    snapshot: SessionSnapshotResponse,
    generation: u32,
    sequence: u32,
) -> SessionStreamMessage {
    SessionStreamMessage::Snapshot {
        session_id: snapshot.session_id.clone(),
        generation,
        sequence,
        snapshot,
    }
}

async fn run_session_socket(mut socket: WebSocket, service: CockpitService, session_id: String) {
    let mut generation = 1u32;
    let mut sequence = 1u32;
    let mut retry = 0u32;

    loop {
        let snapshot = tokio::select! {
            result = service.session_snapshot(&session_id) => result,
            incoming = socket.recv() => {
                if client_closed(incoming, &mut socket).await { return; }
                continue;
            }
        };
        let snapshot = match snapshot {
            Ok(snapshot) if snapshot.session_id == session_id => snapshot,
            Ok(_) => {
                if !send_session_failure(
                    &mut socket,
                    &session_id,
                    &mut generation,
                    &mut sequence,
                    true,
                    "session_snapshot_mismatch",
                    "Session snapshot unavailable",
                )
                .await
                {
                    return;
                }
                if !reconnect_wait(&mut socket, &mut generation, &mut sequence, &mut retry).await {
                    return;
                }
                continue;
            }
            Err(error) => {
                if !send_session_failure(
                    &mut socket,
                    &session_id,
                    &mut generation,
                    &mut sequence,
                    true,
                    &error.code,
                    &error.message,
                )
                .await
                {
                    return;
                }
                if !reconnect_wait(&mut socket, &mut generation, &mut sequence, &mut retry).await {
                    return;
                }
                continue;
            }
        };
        let snapshot_message = session_snapshot_message(snapshot.clone(), generation, sequence);
        if !send_session(
            &mut socket,
            &mut generation,
            &mut sequence,
            snapshot_message,
        )
        .await
        {
            return;
        }

        let subscription = tokio::select! {
            result = service.subscribe_session(&session_id, &snapshot) => result,
            incoming = socket.recv() => {
                if client_closed(incoming, &mut socket).await { return; }
                continue;
            }
        };
        let SessionSubscription { mut messages } = match subscription {
            Ok(subscription) => subscription,
            Err(error) => {
                if !send_session_failure(
                    &mut socket,
                    &session_id,
                    &mut generation,
                    &mut sequence,
                    true,
                    &error.code,
                    &error.message,
                )
                .await
                {
                    return;
                }
                if !reconnect_wait(&mut socket, &mut generation, &mut sequence, &mut retry).await {
                    return;
                }
                continue;
            }
        };
        // The adapter's successful subscription establishes the live event
        // boundary. Re-read after that boundary so events buffered meanwhile
        // are ordered after an authoritative post-subscription snapshot.
        let post_subscription_snapshot = match service.session_snapshot(&session_id).await {
            Ok(snapshot) if snapshot.session_id == session_id => snapshot,
            Ok(_) => {
                if !send_session_failure(
                    &mut socket,
                    &session_id,
                    &mut generation,
                    &mut sequence,
                    true,
                    "session_snapshot_mismatch",
                    "Session snapshot unavailable",
                )
                .await
                {
                    return;
                }
                if !reconnect_wait(&mut socket, &mut generation, &mut sequence, &mut retry).await {
                    return;
                }
                continue;
            }
            Err(error) => {
                if !send_session_failure(
                    &mut socket,
                    &session_id,
                    &mut generation,
                    &mut sequence,
                    true,
                    &error.code,
                    &error.message,
                )
                .await
                {
                    return;
                }
                if !reconnect_wait(&mut socket, &mut generation, &mut sequence, &mut retry).await {
                    return;
                }
                continue;
            }
        };
        let snapshot_message =
            session_snapshot_message(post_subscription_snapshot, generation, sequence);
        if !send_session(
            &mut socket,
            &mut generation,
            &mut sequence,
            snapshot_message,
        )
        .await
        {
            return;
        }

        loop {
            tokio::select! {
                incoming = socket.recv() => {
                    if client_closed(incoming, &mut socket).await { return; }
                }
                change = messages.recv() => {
                    match change {
                        Some(SessionChange::Changed) => {
                            match service.session_snapshot(&session_id).await {
                                Ok(snapshot) if snapshot.session_id == session_id => {
                                    let snapshot_message =
                                        session_snapshot_message(snapshot, generation, sequence);
                                    if !send_session(
                                        &mut socket,
                                        &mut generation,
                                        &mut sequence,
                                        snapshot_message,
                                    )
                                    .await
                                    {
                                        return;
                                    }
                                }
                                Ok(_) => {
                                    if !send_session_failure(&mut socket, &session_id, &mut generation, &mut sequence, true, "session_snapshot_mismatch", "Session snapshot unavailable").await { return; }
                                    break;
                                }
                                Err(error) => {
                                    if !send_session_failure(&mut socket, &session_id, &mut generation, &mut sequence, true, &error.code, &error.message).await { return; }
                                    break;
                                }
                            }
                        }
                        Some(SessionChange::Stale { code, message }) => {
                            if !send_session_failure(&mut socket, &session_id, &mut generation, &mut sequence, true, &code, &message).await { return; }
                            break;
                        }
                        Some(SessionChange::Disconnected { code, message }) => {
                            if !send_session_failure(&mut socket, &session_id, &mut generation, &mut sequence, false, &code, &message).await { return; }
                            break;
                        }
                        None => {
                            if !send_session_failure(&mut socket, &session_id, &mut generation, &mut sequence, false, "subscription_closed", "Session stream disconnected").await { return; }
                            break;
                        }
                    }
                }
            }
        }
        if !reconnect_wait(&mut socket, &mut generation, &mut sequence, &mut retry).await {
            return;
        }
    }
}

async fn send_session_failure(
    socket: &mut WebSocket,
    session_id: &str,
    generation: &mut u32,
    sequence: &mut u32,
    stale: bool,
    code: &str,
    message: &str,
) -> bool {
    let message = if stale {
        SessionStreamMessage::Stale {
            session_id: session_id.to_owned(),
            generation: *generation,
            sequence: *sequence,
            code: code.to_owned(),
            message: message.to_owned(),
        }
    } else {
        SessionStreamMessage::Disconnected {
            session_id: session_id.to_owned(),
            generation: *generation,
            sequence: *sequence,
            code: code.to_owned(),
            message: message.to_owned(),
        }
    };
    send_session(socket, generation, sequence, message).await
}

async fn reconnect_wait(
    socket: &mut WebSocket,
    generation: &mut u32,
    sequence: &mut u32,
    retry: &mut u32,
) -> bool {
    let Some(next_generation) = generation.checked_add(1) else {
        return false;
    };
    *generation = next_generation;
    *sequence = 1;
    let delay = Duration::from_millis(25u64.saturating_mul(1u64 << (*retry).min(5)));
    *retry = retry.saturating_add(1);
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        incoming = socket.recv() => !client_closed(incoming, socket).await,
    }
}

async fn client_closed(
    incoming: Option<Result<Message, axum::Error>>,
    socket: &mut WebSocket,
) -> bool {
    match incoming {
        None | Some(Err(_)) => true,
        Some(Ok(Message::Close(_))) => true,
        Some(Ok(Message::Ping(payload))) => socket.send(Message::Pong(payload)).await.is_err(),
        Some(Ok(_)) => false,
    }
}

async fn run_terminal_socket(
    mut socket: WebSocket,
    session: TerminalSession,
    request: TerminalOpenRequest,
) {
    let stream_id = session.stream_id.clone();
    let mut messages = session.messages;
    let commands = session.commands;
    let mut last_seq: Option<u64> = None;

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {
                        break;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_TERMINAL_COMMAND_BYTES {
                            let _ = send_terminal_error(&mut socket, &request, &stream_id, "terminal_message_too_large", "Terminal command is too large").await;
                            let _ = socket.send(Message::Close(Some(CloseFrame { code: 1009, reason: "terminal command is too large".into() }))).await;
                            break;
                        }
                        let command = match serde_json::from_str::<TerminalCommand>(text.as_str()) {
                            Ok(command) if command.validate().is_ok() => command,
                            _ => {
                                let _ = send_terminal_error(&mut socket, &request, &stream_id, "invalid_terminal_command", "Invalid terminal command").await;
                                let _ = socket.send(Message::Close(Some(CloseFrame { code: 1003, reason: "invalid terminal command".into() }))).await;
                                break;
                            }
                        };
                        match commands.try_send(command) {
                            Ok(()) => {}
                            Err(_) => {
                                let _ = send_terminal_error(&mut socket, &request, &stream_id, "terminal_backpressure", "Terminal command queue is full").await;
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        let _ = send_terminal_error(&mut socket, &request, &stream_id, "invalid_terminal_command", "Terminal commands must be JSON text").await;
                        let _ = socket.send(Message::Close(Some(CloseFrame { code: 1003, reason: "terminal commands must be text".into() }))).await;
                        break;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                }
            }
            message = messages.recv() => {
                let Some(message) = message else {
                    let disconnected = TerminalStreamMessage::Disconnected {
                        session_id: request.session_id.clone(),
                        pane_id: request.pane_id.clone(),
                        stream_id: stream_id.clone(),
                        code: "terminal_disconnected".to_owned(),
                        message: "Terminal stream disconnected".to_owned(),
                    };
                    let _ = send_json(&mut socket, &disconnected).await;
                    break;
                };
                if let TerminalStreamMessage::Frame { seq, bytes, .. } = &message {
                    let Some(number) = decimal_sequence(seq) else {
                        let _ = send_terminal_error(&mut socket, &request, &stream_id, "terminal_sequence_error", "Invalid terminal sequence").await;
                        break;
                    };
                    if let Some(previous) = last_seq
                        && number != previous.saturating_add(1)
                    {
                        let _ = send_terminal_error(&mut socket, &request, &stream_id, "terminal_sequence_error", "Terminal sequence is not consecutive").await;
                        break;
                    }
                    if BASE64.decode(bytes.as_bytes()).is_err() {
                        let _ = send_terminal_error(&mut socket, &request, &stream_id, "terminal_frame_invalid", "Invalid terminal frame").await;
                        break;
                    }
                    last_seq = Some(number);
                }
                let terminal_end = matches!(
                    &message,
                    TerminalStreamMessage::Closed { .. }
                        | TerminalStreamMessage::Disconnected { .. }
                        | TerminalStreamMessage::Error { .. }
                        | TerminalStreamMessage::Ownership {
                            state: TerminalOwnershipState::Lost,
                            ..
                        }
                );
                if !send_json(&mut socket, &message).await || terminal_end { break; }
            }
        }
    }
    let _ = tokio::time::timeout(
        Duration::from_millis(250),
        commands.send(TerminalCommand::Release),
    )
    .await;
}

fn decimal_sequence(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

async fn send_terminal_error(
    socket: &mut WebSocket,
    request: &TerminalOpenRequest,
    stream_id: &str,
    code: &str,
    message: &str,
) -> bool {
    send_json(
        socket,
        &TerminalStreamMessage::Error {
            session_id: request.session_id.clone(),
            pane_id: request.pane_id.clone(),
            stream_id: stream_id.to_owned(),
            code: code.to_owned(),
            message: message.to_owned(),
        },
    )
    .await
}

/// Bind and run the gateway in the foreground. The listening line is emitted only
/// after the OS has accepted the bind and uses the actual ephemeral port.
pub async fn serve(config: ServerConfig) -> Result<(), ServerError> {
    validate_bind(config.bind)?;
    let static_dir = validate_static_root(&config.static_dir)?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(ServerError::Serve)?;
    let actual = listener.local_addr().map_err(ServerError::Serve)?;
    let router = build_router_with_validated_root(config.service, static_dir, actual);
    println!("listening http://{actual}");
    std::io::stdout().flush().map_err(ServerError::Serve)?;
    axum::serve(listener, router)
        .await
        .map_err(ServerError::Serve)
}

#[cfg(test)]
mod tests {
    use super::{valid_resource_id, valid_session_id};

    #[test]
    fn resource_ids_accept_herdr_qualified_panes() {
        assert!(valid_resource_id("w1A:p1"));
        assert!(valid_resource_id("pane_1-2"));
    }

    #[test]
    fn resource_ids_reject_traversal_and_control_characters() {
        assert!(!valid_resource_id("../pane"));
        assert!(!valid_resource_id("pane/child"));
        assert!(!valid_resource_id("pane\u{0000}"));
    }

    #[test]
    fn session_ids_remain_strict_names() {
        assert!(valid_session_id("session-1"));
        assert!(!valid_session_id("w1A:p1"));
        assert!(!valid_session_id("../session"));
    }
}
