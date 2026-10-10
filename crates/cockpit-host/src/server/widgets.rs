use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{HeaderMap, header::ORIGIN},
    response::{IntoResponse, Response},
    routing::get,
};
use cockpit_core::CockpitService;
use cockpit_protocol::widget::{WIDGET_MAX_SNAPSHOT_BYTES, WidgetWindowReport};
use serde::Serialize;

use crate::{
    browser_runtime::{BrowserRuntime, WidgetEventStream},
    transport::{
        error::{OperationError, StatusPolicy},
        guard::{Missing, require},
        limits::WIDGET_BYTES,
    },
};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new().route("/api/v1/widgets/events", get(events))
}

async fn events(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Extension(owner): Extension<Option<Arc<BrowserRuntime>>>,
) -> Response {
    // This GET upgrades to a bidirectional window-report channel. The enclosing
    // authority guard validates the exact Origin; require its presence as well.
    if !headers.contains_key(ORIGIN) {
        return OperationError::rejected(
            "request_origin_required",
            "Widget streams require the gateway Origin",
        )
        .into_response(StatusPolicy::Service);
    }
    let owner = match require(owner, Missing::Widget) {
        Ok(value) => value,
        Err(error) => return error.into_response(StatusPolicy::Service),
    };
    let stream = match owner.widget_events().await {
        Ok(value) => value,
        Err(error) => return OperationError::from(error).into_response(StatusPolicy::Service),
    };
    ws.max_message_size(WIDGET_BYTES)
        .max_frame_size(WIDGET_BYTES)
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
                    if reports > 20 || text.len() > WIDGET_BYTES { break; }
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
