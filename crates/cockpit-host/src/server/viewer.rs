use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cockpit_core::CockpitService;
use cockpit_protocol::viewer::ViewerOpenRequest;

use super::{
    MAX_MUTATION_REQUEST_BYTES, bad_request, inspection_error, require_origin, valid_resource_id,
    valid_session_id,
};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/viewer-sources",
            get(sources),
        )
        .route("/api/v1/sessions/{session_id}/viewers/open", post(open))
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/release",
            post(release),
        )
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}

async fn sources(
    State(service): State<CockpitService>,
    Path((session, pane)): Path<(String, String)>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&pane) {
        return bad_request("invalid_viewer_request", "Session or pane ID is invalid");
    }
    let viewers = match service.viewers() {
        Ok(viewers) => viewers,
        Err(error) => return inspection_error(error),
    };
    match viewers.sources(&session, &pane).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn open(
    State(service): State<CockpitService>,
    Path(session): Path<String>,
    body: Result<Json<ViewerOpenRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match body {
        Ok(Json(request)) => request,
        Err(_) => {
            return bad_request(
                "invalid_viewer_request",
                "Expected a bounded JSON viewer request with valid fields",
            );
        }
    };
    let viewers = match service.viewers() {
        Ok(viewers) => viewers,
        Err(error) => return inspection_error(error),
    };
    match viewers.open(&session, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn release(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&viewer) {
        return bad_request("invalid_viewer_request", "Session or viewer ID is invalid");
    }
    let viewers = match service.viewers() {
        Ok(viewers) => viewers,
        Err(error) => return inspection_error(error),
    };
    match viewers.release(&session, &viewer).await {
        Ok(()) => Json(()).into_response(),
        Err(error) => inspection_error(error),
    }
}
