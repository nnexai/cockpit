use cockpit_core::CockpitService;
use cockpit_protocol::{
    credentials::{
        ProviderCredentialClearRequest, ProviderCredentialSetRequest, ProviderCredentialStatus,
        ProviderCredentialStatusList,
    },
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request, stream_error};

fn validate_provider_id(provider_id: &str) -> Result<(), ErrorResponse> {
    if provider_id.is_empty()
        || provider_id.len() > 512
        || provider_id.chars().any(char::is_control)
    {
        return Err(stream_error(
            "invalid_credential_request",
            "Expected a bounded JSON credential request with valid fields",
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn cockpit_provider_credentials(
    service: State<'_, CockpitService>,
) -> Result<ProviderCredentialStatusList, ErrorResponse> {
    Ok(service
        .credentials()
        .map_err(inspection_error_response)?
        .statuses()
        .await)
}

// The request is decoded here so a rejected body is never echoed back.
#[tauri::command]
pub async fn cockpit_provider_credential_set(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ProviderCredentialStatus, ErrorResponse> {
    let request: ProviderCredentialSetRequest = decode_request(request, "credential")?;
    validate_provider_id(&request.provider_id)?;
    service
        .credentials()
        .map_err(inspection_error_response)?
        .set(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_provider_credential_clear(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ProviderCredentialStatus, ErrorResponse> {
    let request: ProviderCredentialClearRequest = decode_request(request, "credential")?;
    validate_provider_id(&request.provider_id)?;
    service
        .credentials()
        .map_err(inspection_error_response)?
        .clear(request)
        .await
        .map_err(inspection_error_response)
}
