use cockpit_core::CockpitService;
use cockpit_protocol::quick_ask::{QuickAskActionRequest, QuickAskAvailability, QuickAskStartRequest, QuickAskStatus};
use crate::transport::{Transport, error::OperationError, guard::valid_session_id, limits::{check_serialized_size, MUTATION_BYTES}};

fn check_session(session_id: &str) -> Result<(), OperationError> {
    if valid_session_id(session_id) { Ok(()) } else { Err(OperationError::rejected("invalid_session_id", "Invalid session id")) }
}
pub async fn availability(service: &CockpitService, _: Transport, (): ()) -> Result<QuickAskAvailability, OperationError> {
    Ok(service.quick_ask()?.availability())
}
pub async fn start(service: &CockpitService, _: Transport, (session_id, request): (String, QuickAskStartRequest)) -> Result<QuickAskStatus, OperationError> {
    check_session(&session_id)?;
    check_serialized_size(&request, MUTATION_BYTES).map_err(|_| OperationError::rejected("invalid_quick_ask_request", "Quick ask request exceeds the size limit"))?;
    service.quick_ask()?.start(&session_id, request).await.map_err(Into::into)
}
pub async fn status(service: &CockpitService, _: Transport, (session_id, request_id): (String, String)) -> Result<QuickAskStatus, OperationError> {
    check_session(&session_id)?;
    service.quick_ask()?.status(&session_id, &request_id).await.map_err(Into::into)
}
pub async fn action(service: &CockpitService, _: Transport, (session_id, request): (String, QuickAskActionRequest)) -> Result<QuickAskStatus, OperationError> {
    check_session(&session_id)?;
    check_serialized_size(&request, MUTATION_BYTES).map_err(|_| OperationError::rejected("invalid_quick_ask_request", "Quick ask request exceeds the size limit"))?;
    service.quick_ask()?.action(&session_id, request).await.map_err(Into::into)
}
