use std::{
    convert::Infallible,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[path = "browser_view.rs"]
mod browser_view;
use crate::OrchestrationRuntime;
use crate::browser_runtime::BrowserRuntime;
use crate::transport::{
    error::{OperationError, Rejection, StatusPolicy},
    guard::{valid_resource_id, valid_session_id},
    limits::TERMINAL_COMMAND_BYTES,
    shutdown::{HostShutdown, ShutdownPolicy},
    terminal::decimal_sequence,
};
use axum::{
    Extension, Router,
    body::Body,
    extract::{
        Path as AxumPath, Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, Request, StatusCode, Uri,
        header::{HOST, ORIGIN},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use cockpit_core::{CockpitService, TerminalSession};
use cockpit_protocol::v1::{
    TerminalCommand, TerminalMode, TerminalMouseKind, TerminalOpenRequest, TerminalOwnershipState,
    TerminalStreamMessage, TerminalTargetKind,
};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

mod operations;
mod session_socket;
mod widgets;

use session_socket::session_events;

/// Configuration for the foreground HTTP gateway.
#[derive(Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub static_dir: PathBuf,
    pub service: CockpitService,
    pub browser_runtime: Option<Arc<BrowserRuntime>>,
    pub orchestration_runtime: Option<Arc<OrchestrationRuntime>>,
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
        None,
        None,
    ))
}

fn build_router_with_validated_root(
    service: CockpitService,
    static_dir: PathBuf,
    expected_authority: SocketAddr,
    browser_runtime: Option<Arc<BrowserRuntime>>,
    orchestration_runtime: Option<Arc<OrchestrationRuntime>>,
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

    operations::operation_routes()
        .route("/api/v1/sessions/{session_id}/events", get(session_events))
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/terminal",
            get(terminal_ws),
        )
        .merge(browser_view::routes())
        .merge(widgets::routes())
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found))
        .fallback_service(static_service)
        .with_state(service)
        .layer(Extension(browser_runtime))
        .layer(Extension(orchestration_runtime))
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

async fn enforce_authority(request: Request<Body>, next: Next, allowed: bool) -> Response {
    if !allowed {
        return OperationError::with_rejection(
            Rejection::Forbidden,
            "invalid_request_authority",
            "Request authority is not allowed",
        )
        .into_response(StatusPolicy::Service);
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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalQuery {
    mode: TerminalMode,
    #[serde(default)]
    target_kind: TerminalTargetKind,
    takeover: bool,
    cols: u16,
    rows: u16,
    cell_width_px: u32,
    cell_height_px: u32,
}

async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(service): State<CockpitService>,
    AxumPath((session_id, pane_id)): AxumPath<(String, String)>,
    Query(query): Query<TerminalQuery>,
) -> Response {
    if !valid_session_id(&session_id) || !valid_resource_id(&pane_id) {
        return OperationError::rejected("invalid_terminal_target", "Invalid terminal target")
            .into_response(StatusPolicy::Service);
    }
    if !valid_dimensions(query.cols, query.rows) {
        return OperationError::rejected(
            "invalid_terminal_dimensions",
            "Invalid terminal dimensions",
        )
        .into_response(StatusPolicy::Service);
    }
    let request = TerminalOpenRequest {
        session_id,
        pane_id,
        mode: query.mode,
        target_kind: query.target_kind,
        takeover: query.takeover,
        cols: query.cols,
        rows: query.rows,
        cell_width_px: query.cell_width_px,
        cell_height_px: query.cell_height_px,
    };
    let session = match service.open_terminal(&request).await {
        Ok(session) => session,
        Err(error) => return OperationError::from(error).into_response(StatusPolicy::Service),
    };
    ws.max_message_size(TERMINAL_COMMAND_BYTES)
        .on_upgrade(move |socket| run_terminal_socket(socket, session, request))
        .into_response()
}

fn valid_dimensions(cols: u16, rows: u16) -> bool {
    cols > 0 && rows > 0
}

async fn api_not_found() -> Response {
    OperationError::with_rejection(Rejection::NotFound, "not_found", "API route not found")
        .into_response(StatusPolicy::Service)
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
                        if text.len() > TERMINAL_COMMAND_BYTES {
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
                        let lossy_motion = matches!(
                            &command,
                            TerminalCommand::Mouse {
                                kind: TerminalMouseKind::Moved | TerminalMouseKind::Drag,
                                ..
                            }
                        );
                        if lossy_motion {
                            match commands.try_send(command) {
                                Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
                                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                    let _ = send_terminal_error(&mut socket, &request, &stream_id, "terminal_closed", "Terminal command channel closed").await;
                                    break;
                                }
                            }
                        } else if commands.send(command).await.is_err() {
                            let _ = send_terminal_error(&mut socket, &request, &stream_id, "terminal_closed", "Terminal command channel closed").await;
                            break;
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
                            state:
                                TerminalOwnershipState::Lost
                                | TerminalOwnershipState::Conflict,
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
pub async fn serve(config: ServerConfig) -> Result<(), ServerError> {
    validate_bind(config.bind)?;
    let static_dir = validate_static_root(&config.static_dir)?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(ServerError::Serve)?;
    let actual = listener.local_addr().map_err(ServerError::Serve)?;
    let projects = config.service.projects().ok().cloned();
    let browser_runtime = config.browser_runtime.clone();
    let orchestration_runtime = config.orchestration_runtime.clone();
    if let (Some(orchestration), Some(browser)) = (&orchestration_runtime, &browser_runtime) {
        orchestration.start_owner(browser.is_owner().await).await;
    }
    let router = build_router_with_validated_root(
        config.service,
        static_dir,
        actual,
        browser_runtime.clone(),
        orchestration_runtime.clone(),
    );
    println!("listening http://{actual}");
    std::io::stdout().flush().map_err(ServerError::Serve)?;
    let shutdown = HostShutdown::new(orchestration_runtime, projects, browser_runtime);
    let graceful_shutdown = shutdown.clone();
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            graceful_shutdown.run(ShutdownPolicy::Gateway).await;
        })
        .await
        .map_err(ServerError::Serve);
    shutdown.orchestration().await;
    result
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
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
