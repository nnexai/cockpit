use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use cockpit_core::{CockpitService, InspectionError};
use cockpit_protocol::{notes::NotesRequest, v1::ErrorResponse};

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/notes", post(execute))
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .route_layer(middleware::from_fn(super::require_origin))
}

async fn execute(
    State(service): State<CockpitService>,
    body: Result<Json<NotesRequest>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(request)) => request,
        Err(error) => {
            let (code, message) = if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                ("notes_too_large", "Notes request exceeds the 4 MiB limit")
            } else {
                (
                    "notes_usage",
                    "Expected a bounded JSON Notes request with valid fields",
                )
            };
            return notes_error(InspectionError::new(code, message));
        }
    };
    let notes = match service.notes() {
        Ok(notes) => notes,
        Err(error) => return notes_error(error),
    };
    match notes.execute(request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => notes_error(error),
    }
}

fn notes_error(error: InspectionError) -> Response {
    let status = match error.code.as_str() {
        "notes_usage"
        | "notes_target_required"
        | "notes_unbound"
        | "notes_invalid_input"
        | "notes_too_large"
        | "notes_invalid_encoding"
        | "notes_unsafe_path"
        | "notes_invalid_target" => StatusCode::BAD_REQUEST,
        "notes_not_found" | "notes_not_on_board" => StatusCode::NOT_FOUND,
        "notes_conflict"
        | "notes_decision_replaced"
        | "notes_already_bound"
        | "notes_todo_ambiguous"
        | "notes_todo_malformed"
        | "notes_todo_has_children" => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        Json(ErrorResponse {
            code: error.code,
            message: error.message,
        }),
    )
        .into_response()
}
