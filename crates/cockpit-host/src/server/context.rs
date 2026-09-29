use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use cockpit_core::{CockpitService, context_search::ContextSearchService};
use cockpit_protocol::context::{
    ContextDirectoryRequest, ContextDocumentRequest, ContextFileIndexRequest,
};
use cockpit_protocol::context_search::{ContextInvalidationRequest, ContextSearchRequest};
use serde::de::DeserializeOwned;

use super::{
    MAX_MUTATION_REQUEST_BYTES, bad_request, inspection_error, require_origin, valid_resource_id,
    valid_session_id,
};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/files",
            post(files),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/directory",
            post(directory),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/document",
            post(document),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/search",
            post(search),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/invalidate",
            post(invalidate),
        )
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}

fn request<T: DeserializeOwned>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| {
        bad_request(
            "invalid_context_request",
            "Expected a bounded JSON context request with valid fields",
        )
    })
}

fn valid_viewer(session: &str, viewer: &str) -> bool {
    valid_session_id(session) && valid_resource_id(viewer)
}


async fn directory(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<ContextDirectoryRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_context_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let contexts = match service.contexts() {
        Ok(contexts) => contexts,
        Err(error) => return inspection_error(error),
    };
    match contexts.directory(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn files(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<ContextFileIndexRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_context_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let contexts = match service.contexts() {
        Ok(contexts) => contexts,
        Err(error) => return inspection_error(error),
    };
    match contexts.file_index(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn document(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<ContextDocumentRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_context_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let contexts = match service.contexts() {
        Ok(contexts) => contexts,
        Err(error) => return inspection_error(error),
    };
    match contexts.document(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn search(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<ContextSearchRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_context_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let contexts = match service.contexts() {
        Ok(contexts) => contexts.clone(),
        Err(error) => return inspection_error(error),
    };
    match ContextSearchService::new(contexts)
        .search(&session, &viewer, &request)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn invalidate(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<ContextInvalidationRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_context_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let contexts = match service.contexts() {
        Ok(contexts) => contexts.clone(),
        Err(error) => return inspection_error(error),
    };
    match ContextSearchService::new(contexts)
        .invalidate(&session, &viewer, &request)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

