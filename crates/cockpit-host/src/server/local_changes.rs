use std::time::Duration;

use axum::{
    Extension,
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::{IntoResponse, Response},
};
use cockpit_core::CockpitService;
use cockpit_protocol::local_changes::{LocalChangeAck, LocalChangeEvent, LocalChangeKind, LocalChangeReason, LocalChangeScope, MAX_ACK_FRAME, MAX_SCOPE_FRAME};
use uuid::Uuid;

use crate::transport::local_changes::{self, LocalChangeSink};

pub(super) async fn events(
    ws: WebSocketUpgrade,
    State(service): State<CockpitService>,
    Extension(sockets): Extension<super::ListenerSockets>,
) -> Response {
    let token = sockets.tasks.token();
    ws.max_message_size(MAX_SCOPE_FRAME).max_frame_size(MAX_SCOPE_FRAME).on_upgrade(move |socket| async move {
        let _token = token;
        tokio::select! {
            _ = sockets.cancel.cancelled() => {},
            _ = serve(socket, service) => {},
        }
    }).into_response()
}

async fn serve(mut socket: WebSocket, service: CockpitService) {
    let scope = match tokio::time::timeout(Duration::from_secs(5), socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) if text.len() <= MAX_SCOPE_FRAME => {
            match serde_json::from_str::<LocalChangeScope>(&text) {
                Ok(scope) if scope.validate().is_ok() => scope,
                _ => return,
            }
        },
        _ => return,
    };
    let stream_id = Uuid::new_v4().simple().to_string();
    let mut sink = SocketSink { socket };
    let permit = local_changes::admit();
    let subscription = if permit.is_some() {
        tokio::select! {
            result = service.subscribe_local_changes(&scope) => result.ok(),
            _ = sink.socket.recv() => return,
        }
    } else { None };
    match subscription {
        Some(subscription) => {
            let _permit = permit;
            local_changes::pump(subscription, stream_id, &mut sink).await;
        },
        None => {
            let event = LocalChangeEvent {
                stream_id, sequence: 1, kind: LocalChangeKind::Unavailable, topics: Vec::new(),
                reason: Some(LocalChangeReason { code: "watch_unavailable".into(), message: "Automatic updates are unavailable".into() }),
            };
            if matches!(tokio::time::timeout(Duration::from_secs(5), sink.send(event)).await, Ok(true)) {
                // Give the consumer time to display stale state before closing.
                let _ = tokio::time::timeout(Duration::from_secs(30), sink.incoming()).await;
            }
        },
    }
}

struct SocketSink { socket: WebSocket }

impl LocalChangeSink for SocketSink {
    async fn send(&mut self, event: LocalChangeEvent) -> bool {
        let Ok(text) = serde_json::to_string(&event) else { return false; };
        self.socket.send(Message::Text(text.into())).await.is_ok()
    }

    async fn incoming(&mut self) -> Option<LocalChangeAck> {
        loop {
            match self.socket.recv().await {
                Some(Ok(Message::Text(text))) if text.len() <= MAX_ACK_FRAME => return serde_json::from_str(&text).ok(),
                Some(Ok(Message::Ping(payload))) => {
                    if self.socket.send(Message::Pong(payload)).await.is_err() { return None; }
                },
                Some(Ok(Message::Pong(_))) => {},
                _ => return None,
            }
        }
    }
}
