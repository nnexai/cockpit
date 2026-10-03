use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Json, Router,
    extract::{
        DefaultBodyLimit,
        rejection::JsonRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, header::ORIGIN},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cockpit_core::CockpitService;
use cockpit_protocol::widget::{
    WIDGET_MAX_SELECTION_BYTES, WIDGET_MAX_SNAPSHOT_BYTES, WidgetContentRequest,
    WidgetRemoveRequest, WidgetSelectRequest, WidgetWindowReport,
};
use serde::{Serialize, de::DeserializeOwned};

use super::{bad_request, inspection_error, require_origin};
use crate::browser_runtime::{BrowserRuntime, WidgetEventStream};

const MAX_REQUEST_BYTES: usize = 4096;

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/widgets/events", get(events))
        .merge(
            Router::new()
                .route("/api/v1/widgets/content", post(content))
                .route("/api/v1/widgets/remove", post(remove))
                .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
                .merge(
                    Router::new()
                        .route("/api/v1/widgets/select", post(select))
                        .layer(DefaultBodyLimit::max(
                            2 * WIDGET_MAX_SELECTION_BYTES + MAX_REQUEST_BYTES,
                        )),
                )
                .route_layer(middleware::from_fn(require_origin)),
        )
}

fn request<T: DeserializeOwned>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value)
        .map_err(|_| bad_request("widget_usage", "Malformed or oversized widget request"))
}

fn runtime(value: Option<Arc<BrowserRuntime>>) -> Result<Arc<BrowserRuntime>, Response> {
    value.ok_or_else(|| bad_request("widget_no_owner", "Widget runtime is not configured"))
}

async fn content(
    Extension(owner): Extension<Option<Arc<BrowserRuntime>>>,
    body: Result<Json<WidgetContentRequest>, JsonRejection>,
) -> Response {
    let owner = match runtime(owner) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let request = match request(body) {
        Ok(value) => value,
        Err(error) => return error,
    };
    match owner.widget_content(request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn remove(
    Extension(owner): Extension<Option<Arc<BrowserRuntime>>>,
    body: Result<Json<WidgetRemoveRequest>, JsonRejection>,
) -> Response {
    let owner = match runtime(owner) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let request = match request(body) {
        Ok(value) => value,
        Err(error) => return error,
    };
    match owner.widget_remove(request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn select(
    Extension(owner): Extension<Option<Arc<BrowserRuntime>>>,
    body: Result<Json<WidgetSelectRequest>, JsonRejection>,
) -> Response {
    let owner = match runtime(owner) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let request = match request(body) {
        Ok(value) => value,
        Err(error) => return error,
    };
    match owner.widget_select(request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn events(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Extension(owner): Extension<Option<Arc<BrowserRuntime>>>,
) -> Response {
    // This GET upgrades to a bidirectional window-report channel. The enclosing
    // authority guard validates the exact Origin; require its presence as well.
    if !headers.contains_key(ORIGIN) {
        return bad_request(
            "request_origin_required",
            "Widget streams require the gateway Origin",
        );
    }
    let owner = match runtime(owner) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let stream = match owner.widget_events().await {
        Ok(value) => value,
        Err(error) => return inspection_error(error),
    };
    ws.max_message_size(MAX_REQUEST_BYTES)
        .max_frame_size(MAX_REQUEST_BYTES)
        .on_upgrade(move |socket| relay(socket, owner, stream))
        .into_response()
}

async fn send<T: Serialize>(socket: &mut WebSocket, value: &T) -> bool {
    let Ok(json) = serde_json::to_string(value) else {
        return false;
    };
    json.len() <= WIDGET_MAX_SNAPSHOT_BYTES && socket.send(Message::Text(json.into())).await.is_ok()
}

async fn relay(mut socket: WebSocket, owner: Arc<BrowserRuntime>, mut stream: WidgetEventStream) {
    if !send(&mut socket, &stream.snapshot).await {
        return;
    }
    let mut report_period = tokio::time::Instant::now();
    let mut reports = 0_u32;
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if report_period.elapsed() >= Duration::from_secs(1) {
                        report_period = tokio::time::Instant::now();
                        reports = 0;
                    }
                    reports += 1;
                    if reports > 20 || text.len() > MAX_REQUEST_BYTES { break; }
                    let Ok(report) = serde_json::from_str::<WidgetWindowReport>(&text) else { break };
                    if owner.widget_report(&stream.window_id, report).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(payload))) => if socket.send(Message::Pong(payload)).await.is_err() { break; },
                Some(Ok(Message::Pong(_))) => {},
                _ => break,
            },
            next = stream.events.recv() => match next {
                Ok(event) => if !send(&mut socket, &event).await { break; },
                Err(_) => break, // Lag needs a new snapshot, never a partial continuation.
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
    // Keep the whole stream alive until here: Drop deregisters this window and
    // closes an observer relay rather than leaking owner displayed state.
}
