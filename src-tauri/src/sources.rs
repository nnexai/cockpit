use super::{inspection_error_response, requests::decode_request};
use cockpit_core::CockpitService;
use cockpit_protocol::{
    sources::{SourceImportRequest, SourceImportResponse, SourceListRequest, SourceRefreshRequest},
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;
#[tauri::command]
pub async fn cockpit_source_import(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<SourceImportResponse, ErrorResponse> {
    let request: SourceImportRequest = decode_request(request, "source import")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .import_source(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}
#[tauri::command]
pub async fn cockpit_source_refresh(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<SourceImportResponse, ErrorResponse> {
    let request: SourceRefreshRequest = decode_request(request, "source refresh")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .refresh_source(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}
#[tauri::command]
pub async fn cockpit_source_list(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<SourceImportResponse, ErrorResponse> {
    let request: SourceListRequest = decode_request(request, "source list")?;
    service
        .contexts()
        .map_err(inspection_error_response)?
        .list_sources(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}
