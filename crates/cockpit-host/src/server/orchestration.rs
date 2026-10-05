use super::{
    MAX_MUTATION_REQUEST_BYTES, bad_request, inspection_error, require_origin, valid_session_id,
};
use crate::OrchestrationRuntime;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cockpit_core::CockpitService;
use cockpit_protocol::orchestration::*;
use serde::{Deserialize, de::DeserializeOwned};
use std::sync::Arc;

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/sessions/{session_id}/orchestration", get(snapshot))
        .route(
            "/api/v1/sessions/{session_id}/orchestration/mutations",
            post(mutate),
        )
        .route("/api/v1/orchestration/wait", post(wait))
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotQuery {
    root_id: Option<String>,
}
fn request<T: DeserializeOwned>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| {
        bad_request(
            "invalid_orchestration_request",
            "Expected a bounded JSON orchestration request with valid fields",
        )
    })
}
fn runtime(
    value: Option<Arc<OrchestrationRuntime>>,
) -> Result<Arc<OrchestrationRuntime>, Response> {
    value.ok_or_else(|| {
        bad_request(
            "orchestration_unavailable",
            "Orchestration is not configured",
        )
    })
}
async fn snapshot(
    Extension(value): Extension<Option<Arc<OrchestrationRuntime>>>,
    Path(session_id): Path<String>,
    query: Result<Query<SnapshotQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let query = match query {
        Ok(Query(query)) => query,
        Err(_) => {
            return bad_request(
                "invalid_orchestration_request",
                "Invalid orchestration snapshot query",
            );
        }
    };
    let runtime = match runtime(value) {
        Ok(runtime) => runtime,
        Err(error) => return error,
    };
    match runtime
        .snapshot(&OrchestrationSnapshotRequest {
            session_id,
            root_id: query.root_id,
        })
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
async fn mutate(
    Extension(value): Extension<Option<Arc<OrchestrationRuntime>>>,
    Path(session_id): Path<String>,
    body: Result<Json<OrchestrationMutationRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(error) => return error,
    };
    if request.session_id != session_id {
        return bad_request("session_mismatch", "Path and request session IDs differ");
    }
    let runtime = match runtime(value) {
        Ok(runtime) => runtime,
        Err(error) => return error,
    };
    // The route is protected by an exact Origin guard; no wire field chooses the actor.
    match runtime.mutate(OperatorOrigin::Browser, request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
async fn wait(
    Extension(value): Extension<Option<Arc<OrchestrationRuntime>>>,
    body: Result<Json<OrchestrationWaitRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(error) => return error,
    };
    let runtime = match runtime(value) {
        Ok(runtime) => runtime,
        Err(error) => return error,
    };
    // No service mutex is held across the long poll.
    match runtime.wait(&request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
