use cockpit_core::CockpitService;
use cockpit_protocol::{
    quota::{QuotaStatusRequest, QuotaStatusResponse},
    v1::{
        FocusRequest, FocusResponse, ResourceMutationRequest, ResourceMutationResponse,
        SessionListResponse, SessionSnapshotResponse, SpaceGitActionRequest,
        SpaceGitActionResponse, SpaceGitStatusResponse, StatusResponse,
    },
};

use crate::transport::{
    Transport,
    error::OperationError,
    guard::{valid_resource_id, valid_session_id},
    limits::{MUTATION_BYTES, SerializationLimitError, check_serialized_size},
};

pub async fn status(
    service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<StatusResponse, OperationError> {
    Ok(service.status().await)
}

pub async fn quota_status(
    service: &CockpitService,
    _transport: Transport,
    request: QuotaStatusRequest,
) -> Result<QuotaStatusResponse, OperationError> {
    Ok(service.quota()?.status(request).await)
}

pub async fn sessions(
    service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<SessionListResponse, OperationError> {
    service.sessions().await.map_err(Into::into)
}

pub async fn session_snapshot(
    service: &CockpitService,
    _transport: Transport,
    session_id: String,
) -> Result<SessionSnapshotResponse, OperationError> {
    service
        .session_snapshot(&session_id)
        .await
        .map_err(Into::into)
}

pub async fn space_git_status(
    service: &CockpitService,
    _transport: Transport,
    session_id: String,
) -> Result<SpaceGitStatusResponse, OperationError> {
    service
        .space_git_status(&session_id)
        .await
        .map_err(Into::into)
}

pub async fn space_git_action(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, SpaceGitActionRequest),
) -> Result<SpaceGitActionResponse, OperationError> {
    service
        .space_git_action(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn focus(
    service: &CockpitService,
    transport: Transport,
    (session_id, request): (String, FocusRequest),
) -> Result<FocusResponse, OperationError> {
    if transport == Transport::Gateway
        && (!valid_session_id(&session_id) || !valid_resource_id(&request.target_id))
    {
        return Err(OperationError::rejected(
            "invalid_focus_target",
            "Invalid focus target",
        ));
    }
    service
        .focus(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn mutate(
    service: &CockpitService,
    transport: Transport,
    (session_id, request): (String, ResourceMutationRequest),
) -> Result<ResourceMutationResponse, OperationError> {
    match transport {
        Transport::Gateway => {
            if !valid_session_id(&session_id) {
                return Err(OperationError::rejected(
                    "invalid_session_id",
                    "Invalid session id",
                ));
            }
        }
        Transport::Native => {
            check_serialized_size(&request, MUTATION_BYTES).map_err(|error| match error {
                SerializationLimitError::Invalid(_) => {
                    OperationError::rejected("invalid_mutation_request", "Invalid mutation request")
                }
                SerializationLimitError::TooLarge => OperationError::rejected(
                    "mutation_request_too_large",
                    "Mutation request exceeds the 64 KiB limit",
                ),
            })?;
        }
    }
    service
        .mutate(&session_id, &request)
        .await
        .map_err(Into::into)
}
