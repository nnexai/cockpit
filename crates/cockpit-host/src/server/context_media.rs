use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use cockpit_core::CockpitService;
use cockpit_protocol::context_media::ContextMediaRequest;
use serde::de::DeserializeOwned;

use super::{
    MAX_MUTATION_REQUEST_BYTES, bad_request, inspection_error, require_origin, valid_resource_id,
    valid_session_id,
};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/context/media",
            post(media),
        )
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}

fn request<T: DeserializeOwned>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| {
        bad_request(
            "invalid_context_media_request",
            "Expected a bounded JSON Context media request with valid fields",
        )
    })
}

async fn media(
    State(service): State<CockpitService>,
    Path((session, pane)): Path<(String, String)>,
    body: Result<Json<ContextMediaRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&pane) {
        return bad_request(
            "invalid_context_media_request",
            "Session or pane ID is invalid",
        );
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let contexts = match service.contexts() {
        Ok(contexts) => contexts,
        Err(error) => return inspection_error(error),
    };
    match contexts.media(&session, &pane, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
