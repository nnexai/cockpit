use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use cockpit_core::CockpitService;
use cockpit_protocol::comment_paste::{
    CommentPasteMarkPastedRequest, CommentPastePrepareRequest, CommentPasteSendRequest,
};
use cockpit_protocol::comments::{
    CommentBatchList, CommentBatchMutation, CommentBatchRequest, CommentPreview,
    CommentPreviewRequest, CommentRemoveRequest, CommentUpsertRequest,
};
use serde::de::DeserializeOwned;

use super::{
    MAX_MUTATION_REQUEST_BYTES, bad_request, inspection_error, require_origin, valid_resource_id,
    valid_session_id,
};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/list",
            post(list),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/batch",
            post(batch),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/upsert",
            post(upsert),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/remove",
            post(remove),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/discard",
            post(discard),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/attach",
            post(attach),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/preview",
            post(preview),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/paste-prepare",
            post(paste_prepare),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/paste-send",
            post(paste_send),
        )
        .route(
            "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/paste-mark-pasted",
            post(paste_mark_pasted),
        )
        .layer(DefaultBodyLimit::max(MAX_MUTATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}

fn request<T: DeserializeOwned>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| {
        bad_request(
            "invalid_comments_request",
            "Expected a bounded JSON comments request with valid fields",
        )
    })
}

fn valid_viewer(session: &str, viewer: &str) -> bool {
    valid_session_id(session) && valid_resource_id(viewer)
}

async fn list(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<cockpit_protocol::comments::CommentRequestScope>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.list(&session, &viewer, &request).await {
        Ok(value) => Json::<CommentBatchList>(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn batch(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentBatchRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.batch(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn upsert(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentUpsertRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.upsert(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn remove(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentRemoveRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.remove(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn discard(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentBatchMutation>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.discard(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn attach(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentBatchMutation>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.attach(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn preview(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentPreviewRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.preview(&session, &viewer, &request).await {
        Ok(value) => Json::<CommentPreview>(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn paste_prepare(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentPastePrepareRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.paste_prepare(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn paste_send(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentPasteSendRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.paste_send(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn paste_mark_pasted(
    State(service): State<CockpitService>,
    Path((session, viewer)): Path<(String, String)>,
    body: Result<Json<CommentPasteMarkPastedRequest>, JsonRejection>,
) -> Response {
    if !valid_viewer(&session, &viewer) {
        return bad_request("invalid_comments_request", "Session or viewer ID is invalid");
    }
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let comments = match service.comments() {
        Ok(comments) => comments,
        Err(error) => return inspection_error(error),
    };
    match comments.paste_mark_pasted(&session, &viewer, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => inspection_error(error),
    }
}
