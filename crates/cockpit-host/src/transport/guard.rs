use std::sync::Arc;

use axum::{
    body::Body,
    extract::Request,
    http::{Method, header::ORIGIN},
    middleware::Next,
    response::Response,
};

use super::error::{OperationError, StatusPolicy};

pub fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn valid_resource_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    Browser,
    Widget,
    Orchestration,
}

pub fn require<T>(value: Option<Arc<T>>, missing: Missing) -> Result<Arc<T>, OperationError> {
    value.ok_or_else(|| {
        let (code, message) = match missing {
            Missing::Browser => (
                "browser_runtime_unavailable",
                "Browser runtime is not configured",
            ),
            Missing::Widget => ("widget_no_owner", "Widget runtime is not configured"),
            Missing::Orchestration => (
                "orchestration_unavailable",
                "Orchestration is not configured",
            ),
        };
        OperationError::rejected(code, message)
    })
}

// The enclosing guard verifies the exact bound Host and Origin.
pub async fn require_origin(request: Request<Body>, next: Next) -> Response {
    if request.method() != Method::GET && !request.headers().contains_key(ORIGIN) {
        return OperationError::rejected(
            "request_origin_required",
            "Filesystem requests require the gateway Origin",
        )
        .into_response(StatusPolicy::Service);
    }
    next.run(request).await
}
