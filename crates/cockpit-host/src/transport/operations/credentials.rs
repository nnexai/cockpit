//! Write-only provider credential operations. A set body carries a secret: it is
//! never logged, echoed or included in an error.

use cockpit_core::CockpitService;
use cockpit_protocol::credentials::{
    ProviderCredentialClearRequest, ProviderCredentialSetRequest, ProviderCredentialStatus,
    ProviderCredentialStatusList,
};

use crate::transport::{Transport, error::OperationError};

fn validate_provider_id(provider_id: &str) -> Result<(), OperationError> {
    if provider_id.is_empty()
        || provider_id.len() > 512
        || provider_id.chars().any(char::is_control)
    {
        return Err(OperationError::rejected(
            "invalid_credential_request",
            "Expected a bounded JSON credential request with valid fields",
        ));
    }
    Ok(())
}

pub async fn provider_credentials(
    service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<ProviderCredentialStatusList, OperationError> {
    Ok(service.credentials()?.statuses().await)
}

// The request is decoded by the input adapter so a rejected body is never echoed back.
pub async fn provider_credential_set(
    service: &CockpitService,
    _transport: Transport,
    request: ProviderCredentialSetRequest,
) -> Result<ProviderCredentialStatus, OperationError> {
    validate_provider_id(&request.provider_id)?;
    service
        .credentials()?
        .set(request)
        .await
        .map_err(Into::into)
}

pub async fn provider_credential_clear(
    service: &CockpitService,
    _transport: Transport,
    request: ProviderCredentialClearRequest,
) -> Result<ProviderCredentialStatus, OperationError> {
    validate_provider_id(&request.provider_id)?;
    service
        .credentials()?
        .clear(request)
        .await
        .map_err(Into::into)
}
