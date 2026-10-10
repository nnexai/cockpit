use cockpit_core::CockpitService;
use cockpit_protocol::{
    comment_paste::{
        CommentPasteMarkPastedRequest, CommentPastePrepareRequest, CommentPastePrepareResponse,
        CommentPasteReceipt, CommentPasteSendRequest,
    },
    comments::{
        CommentBatch, CommentBatchList, CommentBatchMutation, CommentBatchRequest, CommentPreview,
        CommentPreviewRequest, CommentRemoveRequest, CommentRequestScope, CommentUpsertRequest,
    },
};

use crate::transport::{Transport, error::OperationError};

pub async fn comments_list(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentRequestScope),
) -> Result<CommentBatchList, OperationError> {
    service
        .comments()?
        .list(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_batch(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentBatchRequest),
) -> Result<CommentBatch, OperationError> {
    service
        .comments()?
        .batch(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_upsert(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentUpsertRequest),
) -> Result<CommentBatch, OperationError> {
    service
        .comments()?
        .upsert(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_remove(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentRemoveRequest),
) -> Result<CommentBatch, OperationError> {
    service
        .comments()?
        .remove(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_discard(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentBatchMutation),
) -> Result<CommentBatchList, OperationError> {
    service
        .comments()?
        .discard(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_attach(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentBatchMutation),
) -> Result<CommentBatch, OperationError> {
    service
        .comments()?
        .attach(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_preview(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentPreviewRequest),
) -> Result<CommentPreview, OperationError> {
    service
        .comments()?
        .preview(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_paste_prepare(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentPastePrepareRequest),
) -> Result<CommentPastePrepareResponse, OperationError> {
    service
        .comments()?
        .paste_prepare(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_paste_send(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentPasteSendRequest),
) -> Result<CommentPasteReceipt, OperationError> {
    service
        .comments()?
        .paste_send(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn comments_paste_mark_pasted(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, CommentPasteMarkPastedRequest),
) -> Result<CommentPasteReceipt, OperationError> {
    service
        .comments()?
        .paste_mark_pasted(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}
