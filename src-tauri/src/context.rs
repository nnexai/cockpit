use cockpit_core::CockpitService;
use cockpit_protocol::{
    context::{
        ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
        ContextFileIndex, ContextFileIndexRequest,
    },
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request};

#[tauri::command]
pub async fn cockpit_context_directory(
    session_id: String,
    viewer_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextDirectory, ErrorResponse> {
    let request: ContextDirectoryRequest = decode_request(request, "context")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .directory(&session_id, &viewer_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_context_file_index(
    session_id: String,
    viewer_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextFileIndex, ErrorResponse> {
    let request: ContextFileIndexRequest = decode_request(request, "context file index")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .file_index(&session_id, &viewer_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_context_document(
    session_id: String,
    viewer_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextDocument, ErrorResponse> {
    let request: ContextDocumentRequest = decode_request(request, "context")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .document(&session_id, &viewer_id, &request)
        .await
        .map_err(inspection_error_response)
}

