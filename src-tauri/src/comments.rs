use cockpit_core::CockpitService;
use cockpit_protocol::comment_paste::{
    CommentPasteMarkPastedRequest, CommentPastePrepareRequest, CommentPastePrepareResponse,
    CommentPasteReceipt, CommentPasteSendRequest,
};
use cockpit_protocol::{
    comments::{
        CommentBatch, CommentBatchList, CommentBatchMutation, CommentBatchRequest, CommentPreview,
        CommentPreviewRequest, CommentRemoveRequest, CommentUpsertRequest,
    },
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request};

#[tauri::command]
pub async fn cockpit_comments_list(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentBatchList, ErrorResponse> {
    let request: cockpit_protocol::comments::CommentRequestScope =
        decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .list(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_batch(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentBatch, ErrorResponse> {
    let request: CommentBatchRequest = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .batch(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_upsert(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentBatch, ErrorResponse> {
    let request: CommentUpsertRequest = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .upsert(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_remove(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentBatch, ErrorResponse> {
    let request: CommentRemoveRequest = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .remove(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_attach(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentBatch, ErrorResponse> {
    let request: CommentBatchMutation = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .attach(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_preview(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentPreview, ErrorResponse> {
    let request: CommentPreviewRequest = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .preview(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_paste_prepare(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentPastePrepareResponse, ErrorResponse> {
    let request: CommentPastePrepareRequest = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .paste_prepare(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_paste_send(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentPasteReceipt, ErrorResponse> {
    let request: CommentPasteSendRequest = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .paste_send(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_comments_paste_mark_pasted(
    session_id: String,
    pane_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<CommentPasteReceipt, ErrorResponse> {
    let request: CommentPasteMarkPastedRequest = decode_request(request, "comments")?;
    service
        .comments()
        .map_err(inspection_error_response)?
        .paste_mark_pasted(&session_id, &pane_id, &request)
        .await
        .map_err(inspection_error_response)
}
