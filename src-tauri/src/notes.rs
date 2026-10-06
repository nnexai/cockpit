use cockpit_core::CockpitService;
use cockpit_protocol::{
    notes::{NotesRequest, NotesResponse},
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

#[tauri::command]
pub async fn cockpit_notes_execute(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<NotesResponse, ErrorResponse> {
    let request: NotesRequest = super::requests::decode_notes_request(request)?;
    service
        .notes()
        .map_err(super::inspection_error_response)?
        .execute(request)
        .await
        .map_err(super::inspection_error_response)
}
