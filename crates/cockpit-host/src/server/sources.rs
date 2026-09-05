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
use cockpit_protocol::sources::{SourceImportRequest, SourceListRequest, SourceRefreshRequest};
pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/sources/import",
            post(import),
        )
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/sources/refresh",
            post(refresh),
        )
        .route(
            "/api/v1/sessions/{session_id}/panes/{pane_id}/sources/list",
            post(list),
        )
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}
async fn import(
    State(service): State<CockpitService>,
    Path((session, pane)): Path<(String, String)>,
    body: Result<Json<SourceImportRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&pane) {
        return bad_request("source_invalid_request", "Invalid session or pane");
    }
    let request = match body {
        Ok(Json(value)) => value,
        Err(_) => {
            return bad_request(
                "source_invalid_request",
                "Expected a bounded source request",
            );
        }
    };
    let contexts = match service.contexts() {
        Ok(value) => value,
        Err(error) => return inspection_error(error),
    };
    match contexts.import_source(&session, &pane, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
async fn refresh(
    State(service): State<CockpitService>,
    Path((session, pane)): Path<(String, String)>,
    body: Result<Json<SourceRefreshRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&pane) {
        return bad_request("source_invalid_request", "Invalid session or pane");
    }
    let request = match body {
        Ok(Json(value)) => value,
        Err(_) => {
            return bad_request(
                "source_invalid_request",
                "Expected a bounded source request",
            );
        }
    };
    let contexts = match service.contexts() {
        Ok(value) => value,
        Err(error) => return inspection_error(error),
    };
    match contexts.refresh_source(&session, &pane, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
async fn list(
    State(service): State<CockpitService>,
    Path((session, pane)): Path<(String, String)>,
    body: Result<Json<SourceListRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session) || !valid_resource_id(&pane) {
        return bad_request("source_invalid_request", "Invalid session or pane");
    }
    let request = match body {
        Ok(Json(value)) => value,
        Err(_) => {
            return bad_request(
                "source_invalid_request",
                "Expected a bounded source request",
            );
        }
    };
    let contexts = match service.contexts() {
        Ok(value) => value,
        Err(error) => return inspection_error(error),
    };
    match contexts.list_sources(&session, &pane, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
