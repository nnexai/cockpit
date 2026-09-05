use super::{
    MAX_MUTATION_REQUEST_BYTES, bad_request, inspection_error, require_origin, valid_resource_id,
    valid_session_id,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use cockpit_core::CockpitService;
use cockpit_protocol::{
    context::{PanePresentation, ReviewLaunchRequest},
    review::{ReviewFileRequest, ReviewSnapshotRequest},
};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/sessions/{session_id}/review/open", post(open))
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/review/snapshot",
            post(snapshot),
        )
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/review/file",
            post(file),
        )
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}
async fn open(
    State(service): State<CockpitService>,
    Path(session): Path<String>,
    body: Result<Json<ReviewLaunchRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) {
        return bad_request("review_invalid_request", "Invalid session");
    }
    let request = match body {
        Ok(Json(value)) => value,
        Err(_) => {
            return bad_request(
                "review_invalid_request",
                "Expected a bounded review launch request",
            );
        }
    };
    let contexts = match service.contexts() {
        Ok(value) => value,
        Err(error) => return inspection_error(error),
    };
    match contexts.open_review(&session, &request).await {
        Ok(value) => Json::<PanePresentation>(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
async fn snapshot(
    State(service): State<CockpitService>,
    Path((session, pane)): Path<(String, String)>,
    body: Result<Json<ReviewSnapshotRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&pane) {
        return bad_request("review_invalid_request", "Invalid session or pane");
    }
    let request = match body {
        Ok(Json(value)) => value,
        Err(_) => {
            return bad_request(
                "review_invalid_request",
                "Expected a bounded review snapshot request",
            );
        }
    };
    let reviews = match service.reviews() {
        Ok(value) => value,
        Err(error) => return inspection_error(error),
    };
    match reviews.snapshot(&session, &pane, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
async fn file(
    State(service): State<CockpitService>,
    Path((session, pane)): Path<(String, String)>,
    body: Result<Json<ReviewFileRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&pane) {
        return bad_request("review_invalid_request", "Invalid session or pane");
    }
    let request = match body {
        Ok(Json(value)) => value,
        Err(_) => {
            return bad_request(
                "review_invalid_request",
                "Expected a bounded review file request",
            );
        }
    };
    let reviews = match service.reviews() {
        Ok(value) => value,
        Err(error) => return inspection_error(error),
    };
    match reviews.file(&session, &pane, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
