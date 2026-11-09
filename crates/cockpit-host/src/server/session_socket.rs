use axum::{
    Extension,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
};
use cockpit_core::CockpitService;
use cockpit_protocol::v1::SessionStreamMessage;

use crate::transport::{
    error::{OperationError, StatusPolicy},
    guard::valid_session_id,
    session_stream::{Incoming, SessionSink, SessionStream, pump},
};

pub(super) async fn session_events(
    ws: WebSocketUpgrade,
    State(service): State<CockpitService>,
    Extension(sockets): Extension<super::ListenerSockets>,
    Path(session_id): Path<String>,
) -> Response {
    if !valid_session_id(&session_id) {
        return OperationError::rejected("invalid_session_id", "Invalid session id")
            .into_response(StatusPolicy::Service);
    }
    let token = sockets.tasks.token();
    ws.on_upgrade(move |socket| async move {
        let _token = token;
        let (stream, first) = SessionStream::gateway(session_id);
        let mut sink = WebSocketSink { socket };
        tokio::select! {
            _ = sockets.cancel.cancelled() => {}
            _ = pump(service, stream, first, None, &mut sink) => {}
        }
    })
    .into_response()
}

struct WebSocketSink {
    socket: WebSocket,
}

impl SessionSink for WebSocketSink {
    async fn send(&mut self, message: SessionStreamMessage) -> bool {
        let Ok(text) = serde_json::to_string(&message) else {
            return false;
        };
        self.socket.send(Message::Text(text.into())).await.is_ok()
    }

    async fn incoming(&mut self) -> Incoming {
        match self.socket.recv().await {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => Incoming::Closed,
            Some(Ok(Message::Ping(payload))) => {
                if self.socket.send(Message::Pong(payload)).await.is_err() {
                    Incoming::Closed
                } else {
                    Incoming::Activity
                }
            }
            Some(Ok(_)) => Incoming::Activity,
        }
    }

    fn cancelled(&self) -> bool {
        false
    }
}
