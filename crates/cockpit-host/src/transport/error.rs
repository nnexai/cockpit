use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use cockpit_core::InspectionError;
use cockpit_protocol::v1::ErrorResponse;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    BadRequest,
    PayloadTooLarge,
    Forbidden,
    NotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusPolicy {
    Service,
    Notes,
    BadRequest,
}

#[derive(Debug)]
pub enum OperationError {
    Rejected {
        rejection: Rejection,
        body: ErrorResponse,
    },
    Service(InspectionError),
}

/// The only transport error-to-status mapping. Native commands use just the body.
pub fn status(policy: StatusPolicy, error: &OperationError) -> StatusCode {
    let code = match error {
        OperationError::Rejected { rejection, .. } => {
            return match rejection {
                Rejection::BadRequest => StatusCode::BAD_REQUEST,
                Rejection::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
                Rejection::Forbidden => StatusCode::FORBIDDEN,
                Rejection::NotFound => StatusCode::NOT_FOUND,
            };
        }
        OperationError::Service(error) => error.code.as_str(),
    };
    match policy {
        StatusPolicy::BadRequest => StatusCode::BAD_REQUEST,
        StatusPolicy::Service if code.starts_with("invalid_") => StatusCode::BAD_REQUEST,
        StatusPolicy::Service => match code {
            "focus_conflict"
            | "terminal_ownership_conflict"
            | "stale_generation"
            | "space_git_target_changed"
            | "space_git_action_ineligible"
            | "space_git_action_in_progress" => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        StatusPolicy::Notes => match code {
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
        },
    }
}

impl OperationError {
    pub fn rejected(code: &str, message: impl Into<String>) -> Self {
        Self::with_rejection(Rejection::BadRequest, code, message)
    }

    pub fn with_rejection(rejection: Rejection, code: &str, message: impl Into<String>) -> Self {
        Self::Rejected {
            rejection,
            body: ErrorResponse {
                code: code.to_owned(),
                message: message.into(),
            },
        }
    }

    pub fn into_response(self, policy: StatusPolicy) -> Response {
        let status = status(policy, &self);
        (status, Json(ErrorResponse::from(self))).into_response()
    }
}

impl From<InspectionError> for OperationError {
    fn from(error: InspectionError) -> Self {
        Self::Service(error)
    }
}

impl From<OperationError> for ErrorResponse {
    fn from(error: OperationError) -> Self {
        match error {
            OperationError::Rejected { body, .. } => body,
            OperationError::Service(error) => Self {
                code: error.code,
                message: error.message,
            },
        }
    }
}
