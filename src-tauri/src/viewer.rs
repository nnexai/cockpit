use cockpit_core::CockpitService;
use cockpit_protocol::{
    v1::ErrorResponse,
    viewer::{ViewerContext, ViewerOpenRequest, ViewerSourceOptions},
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request};

#[tauri::command]
pub async fn cockpit_viewer_sources(
    session_id: String,
    pane_id: String,
    service: State<'_, CockpitService>,
) -> Result<ViewerSourceOptions, ErrorResponse> {
    service
        .viewers()
        .map_err(inspection_error_response)?
        .sources(&session_id, &pane_id)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_viewer_open(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ViewerContext, ErrorResponse> {
    let request: ViewerOpenRequest = decode_request(request, "viewer")?;
    service
        .viewers()
        .map_err(inspection_error_response)?
        .open(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_viewer_release(
    session_id: String,
    viewer_id: String,
    service: State<'_, CockpitService>,
) -> Result<(), ErrorResponse> {
    service
        .viewers()
        .map_err(inspection_error_response)?
        .release(&session_id, &viewer_id)
        .await
        .map_err(inspection_error_response)
}
