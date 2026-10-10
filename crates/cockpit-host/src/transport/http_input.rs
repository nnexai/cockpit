use axum::{
    Json,
    extract::{
        Path, Query,
        rejection::{JsonRejection, QueryRejection},
    },
    http::StatusCode,
};
use cockpit_core::InspectionError;
use cockpit_protocol::notes::NotesRequest;

use super::{
    error::{OperationError, Rejection},
    guard::{valid_resource_id, valid_session_id},
};

#[derive(Clone, Copy)]
pub struct Reject(pub &'static str, pub &'static str);

impl Reject {
    fn error(self) -> OperationError {
        OperationError::rejected(self.0, self.1)
    }
}

pub fn none() -> impl FnOnce(()) -> Result<(), OperationError> {
    |()| Ok(())
}

pub fn json<T>() -> impl FnOnce((Json<T>,)) -> Result<T, OperationError> {
    |(Json(value),)| Ok(value)
}

pub fn query<T>() -> impl FnOnce((Query<T>,)) -> Result<T, OperationError> {
    |(Query(value),)| Ok(value)
}

pub fn session(reject: Reject) -> impl FnOnce((Path<String>,)) -> Result<String, OperationError> {
    move |(Path(session),)| {
        if !valid_session_id(&session) {
            return Err(reject.error());
        }
        Ok(session)
    }
}

pub fn pair(
    reject: Reject,
) -> impl FnOnce((Path<(String, String)>,)) -> Result<(String, String), OperationError> {
    move |(Path((session, resource)),)| {
        if !valid_session_id(&session) || !valid_resource_id(&resource) {
            return Err(reject.error());
        }
        Ok((session, resource))
    }
}

/// Workspace operation ids are passed through: only the session path is checked.
pub fn session_pair_path(
    reject: Reject,
) -> impl FnOnce((Path<(String, String)>,)) -> Result<(String, String), OperationError> {
    move |(Path((session, id)),)| {
        if !valid_session_id(&session) {
            return Err(reject.error());
        }
        Ok((session, id))
    }
}

pub fn body<T>(
    reject: Reject,
) -> impl FnOnce((Result<Json<T>, JsonRejection>,)) -> Result<T, OperationError> {
    move |(body,)| body.map(|Json(value)| value).map_err(|_| reject.error())
}

/// Check the path before inspecting a mapped body rejection.
pub fn session_body<T>(
    path: Reject,
    body: Reject,
) -> impl FnOnce((Path<String>, Result<Json<T>, JsonRejection>)) -> Result<(String, T), OperationError>
{
    move |(Path(session), request)| {
        if !valid_session_id(&session) {
            return Err(path.error());
        }
        let value = request.map(|Json(value)| value).map_err(|_| body.error())?;
        Ok((session, value))
    }
}

pub fn pair_body<T>(
    path: Reject,
    body: Reject,
) -> impl FnOnce(
    (Path<(String, String)>, Result<Json<T>, JsonRejection>),
) -> Result<(String, String, T), OperationError> {
    move |(Path((session, resource)), request)| {
        if !valid_session_id(&session) || !valid_resource_id(&resource) {
            return Err(path.error());
        }
        let value = request.map(|Json(value)| value).map_err(|_| body.error())?;
        Ok((session, resource, value))
    }
}

/// Space Git alone preserves an oversized-body 413 in its mapped rejection.
pub fn session_sized_body<T>(
    path: Reject,
    body: Reject,
) -> impl FnOnce((Path<String>, Result<Json<T>, JsonRejection>)) -> Result<(String, T), OperationError>
{
    move |(Path(session), request)| {
        if !valid_session_id(&session) {
            return Err(path.error());
        }
        let value = request.map(|Json(value)| value).map_err(|error| {
            let rejection = if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                Rejection::PayloadTooLarge
            } else {
                Rejection::BadRequest
            };
            OperationError::with_rejection(rejection, body.0, body.1)
        })?;
        Ok((session, value))
    }
}

pub fn notes_body()
-> impl FnOnce((Result<Json<NotesRequest>, JsonRejection>,)) -> Result<NotesRequest, OperationError>
{
    |(request,)| {
        request.map(|Json(value)| value).map_err(|error| {
            let (code, message) = if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                ("notes_too_large", "Notes request exceeds the 4 MiB limit")
            } else {
                (
                    "notes_usage",
                    "Expected a bounded JSON Notes request with valid fields",
                )
            };
            InspectionError::new(code, message).into()
        })
    }
}

/// The registry runs the widget owner guard before invoking this body adapter.
pub fn widget_body<T>(
    reject: Reject,
) -> impl FnOnce((Result<Json<T>, JsonRejection>,)) -> Result<T, OperationError> {
    body(reject)
}

pub fn mapped_query<T>(
    reject: Reject,
) -> impl FnOnce((Result<Query<T>, QueryRejection>,)) -> Result<T, OperationError> {
    move |(query,)| query.map(|Query(value)| value).map_err(|_| reject.error())
}

/// Default Json extraction already happened; operations own post-decode checks.
pub fn session_json<T>()
-> impl FnOnce((Path<String>, Json<T>)) -> Result<(String, T), OperationError> {
    |(Path(session), Json(value))| Ok((session, value))
}

pub fn validated_json<T>(
    validate: fn(&T) -> Result<(), OperationError>,
) -> impl FnOnce((Json<T>,)) -> Result<T, OperationError> {
    move |(Json(value),)| {
        validate(&value)?;
        Ok(value)
    }
}

pub fn id_path() -> impl FnOnce((Path<String>,)) -> Result<String, OperationError> {
    |(Path(id),)| Ok(id)
}
