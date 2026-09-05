use cockpit_core::CockpitService;
use cockpit_protocol::{
    context_media::{ContextMedia, ContextMediaRequest},
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request};

#[tauri::command]
pub async fn cockpit_context_media(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextMedia, ErrorResponse> {
    let request: ContextMediaRequest = decode_request(request, "context media")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .media(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}
