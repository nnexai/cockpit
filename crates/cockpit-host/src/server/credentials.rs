//! Write-only provider credential routes. A set body carries a secret: it is
//! never logged, echoed or included in an error.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cockpit_core::CockpitService;
use cockpit_protocol::credentials::{ProviderCredentialClearRequest, ProviderCredentialSetRequest};
use serde::de::DeserializeOwned;

use super::{bad_request, inspection_error, require_origin};

const MAX_CREDENTIAL_REQUEST_BYTES: usize = 16 * 1024;

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/provider-credentials", get(statuses))
        .route("/api/v1/provider-credentials/set", post(set))
        .route("/api/v1/provider-credentials/clear", post(clear))
        .layer(DefaultBodyLimit::max(MAX_CREDENTIAL_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}

fn invalid_request() -> Response {
    bad_request(
        "invalid_credential_request",
        "Expected a bounded JSON credential request with valid fields",
    )
}

/// Rejections are dropped without inspection: their text may quote the body.
fn request<T: DeserializeOwned>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| invalid_request())
}

fn valid_provider_id(provider_id: &str) -> bool {
    !provider_id.is_empty() && provider_id.len() <= 512 && !provider_id.chars().any(char::is_control)
}

async fn statuses(State(service): State<CockpitService>) -> Response {
    match service.credentials() {
        Ok(credentials) => Json(credentials.statuses().await).into_response(),
        Err(error) => inspection_error(error),
    }
}

async fn set(
    State(service): State<CockpitService>,
    body: Result<Json<ProviderCredentialSetRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if !valid_provider_id(&request.provider_id) {
        return invalid_request();
    }
    match service.credentials() {
        Ok(credentials) => match credentials.set(request).await {
            Ok(status) => Json(status).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn clear(
    State(service): State<CockpitService>,
    body: Result<Json<ProviderCredentialClearRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if !valid_provider_id(&request.provider_id) {
        return invalid_request();
    }
    match service.credentials() {
        Ok(credentials) => match credentials.clear(request).await {
            Ok(status) => Json(status).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}
