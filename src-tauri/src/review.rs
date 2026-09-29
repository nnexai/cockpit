use super::{inspection_error_response, requests::decode_request};
use cockpit_core::CockpitService;
use cockpit_protocol::{
    review::{ReviewFileDiff, ReviewFileRequest, ReviewSnapshot, ReviewSnapshotRequest},
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;
#[tauri::command]
pub async fn cockpit_review_snapshot(
    session_id: String,
    viewer_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ReviewSnapshot, ErrorResponse> {
    let request: ReviewSnapshotRequest = decode_request(request, "review snapshot")?;
    service
        .reviews()
        .map_err(inspection_error_response)?
        .snapshot(&session_id, &viewer_id, &request)
        .await
        .map_err(inspection_error_response)
}
#[tauri::command]
pub async fn cockpit_review_file(
    session_id: String,
    viewer_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ReviewFileDiff, ErrorResponse> {
    let request: ReviewFileRequest = decode_request(request, "review file")?;
    service
        .reviews()
        .map_err(inspection_error_response)?
        .file(&session_id, &viewer_id, &request)
        .await
        .map_err(inspection_error_response)
}
