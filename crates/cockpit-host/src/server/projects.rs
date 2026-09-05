use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cockpit_core::CockpitService;
use cockpit_protocol::projects::{
    WorkspaceOperationRequest, WorkspaceReconcileRequest, WorkspaceSetupRequest,
};
use cockpit_protocol::project_teardown::{
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownPreviewRequest,
};
use serde::de::DeserializeOwned;

use super::{
    MAX_MUTATION_REQUEST_BYTES, bad_request, inspection_error, require_origin, valid_session_id,
};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/project/configuration", get(configuration))
        .route("/api/v1/project/repositories", get(repositories))
        .route("/api/v1/sessions/{session_id}/workspace-plans", post(plan))
        .route(
            "/api/v1/sessions/{session_id}/workspace-operations",
            post(start),
        )
        .route(
            "/api/v1/sessions/{session_id}/workspace-operations/{operation_id}",
            get(operation),
        )
        .route(
            "/api/v1/sessions/{session_id}/workspace-operations/resume",
            post(resume),
        )
        .route(
            "/api/v1/sessions/{session_id}/workspace-operations/cancel",
            post(cancel),
        )
        .route(
            "/api/v1/sessions/{session_id}/workspace-operations/reconcile",
            post(reconcile),
        )
        .route(
            "/api/v1/sessions/{session_id}/workspace-teardown/preview",
            post(teardown_preview),
        )
        .route(
            "/api/v1/sessions/{session_id}/workspace-teardown/execute",
            post(teardown_execute),
        )
        .route(
            "/api/v1/sessions/{session_id}/workspace-teardown/recoveries",
            get(teardown_recoveries),
        )
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}

fn request<T: DeserializeOwned>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| {
        bad_request(
            "invalid_project_request",
            "Expected a bounded JSON project request with valid fields",
        )
    })
}

async fn configuration(State(service): State<CockpitService>) -> Response {
    match service.projects() {
        Ok(projects) => Json(projects.configuration()).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn repositories(State(service): State<CockpitService>) -> Response {
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.repositories().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn plan(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
    body: Result<Json<WorkspaceSetupRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.plan(&session_id, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn start(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
    body: Result<Json<WorkspaceOperationRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.start(&session_id, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn operation(
    State(service): State<CockpitService>,
    Path((session_id, operation_id)): Path<(String, String)>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.get(&session_id, &operation_id).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn resume(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
    body: Result<Json<WorkspaceOperationRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.resume(&session_id, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn cancel(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
    body: Result<Json<WorkspaceOperationRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.cancel(&session_id, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn reconcile(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
    body: Result<Json<WorkspaceReconcileRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.reconcile(&session_id, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn teardown_preview(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
    body: Result<Json<WorkspaceTeardownPreviewRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.teardown_preview(&session_id, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn teardown_execute(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
    body: Result<Json<WorkspaceTeardownExecuteRequest>, JsonRejection>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.teardown_execute(&session_id, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn teardown_recoveries(
    State(service): State<CockpitService>,
    Path(session_id): Path<String>,
) -> Response {
    if !valid_session_id(&session_id) {
        return bad_request("invalid_session_id", "Session ID is invalid");
    }
    let projects = match service.projects() {
        Ok(projects) => projects,
        Err(error) => return inspection_error(error),
    };
    match projects.teardown_recoveries(&session_id) {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
