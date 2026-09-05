use cockpit_core::CockpitService;
use cockpit_protocol::{
    context::{
        ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
        ContextLaunchRequest, PanePresentation,
    },
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request};

#[tauri::command]
pub async fn cockpit_pane_presentation(
    session_id: String,
    pane_id: String,
    service: State<'_, CockpitService>,
) -> Result<PanePresentation, ErrorResponse> {
    service
        .contexts()
        .map_err(inspection_error_response)?
        .inspect_pane(&session_id, &pane_id)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_context_directory(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextDirectory, ErrorResponse> {
    let request: ContextDirectoryRequest = decode_request(request, "context")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .directory(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_context_document(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextDocument, ErrorResponse> {
    let request: ContextDocumentRequest = decode_request(request, "context")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .document(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_context_open(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<PanePresentation, ErrorResponse> {
    let request: ContextLaunchRequest = decode_request(request, "context")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .open(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}
