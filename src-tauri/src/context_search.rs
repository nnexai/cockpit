use cockpit_core::{CockpitService, context_search::ContextSearchService};
use cockpit_protocol::{
    context_search::{
        ContextInvalidationRequest, ContextInvalidationResponse, ContextSearchRequest,
        ContextSearchResponse,
    },
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request};

#[tauri::command]
pub async fn cockpit_context_search(
    session_id: String,
    viewer_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextSearchResponse, ErrorResponse> {
    let request: ContextSearchRequest = decode_request(request, "context search")?;
    let contexts = service
        .contexts()
        .map_err(inspection_error_response)?
        .clone();
    ContextSearchService::new(contexts)
        .search(&session_id, &viewer_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_context_invalidate(
    session_id: String,
    viewer_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextInvalidationResponse, ErrorResponse> {
    let request: ContextInvalidationRequest = decode_request(request, "context invalidation")?;
    let contexts = service
        .contexts()
        .map_err(inspection_error_response)?
        .clone();
    ContextSearchService::new(contexts)
        .invalidate(&session_id, &viewer_id, &request)
        .await
        .map_err(inspection_error_response)
}
