use cockpit_core::CockpitService;
use cockpit_protocol::review::{
    ReviewFileDiff, ReviewFileRequest, ReviewSnapshot, ReviewSnapshotRequest,
};

use crate::transport::{Transport, error::OperationError};

pub async fn review_snapshot(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ReviewSnapshotRequest),
) -> Result<ReviewSnapshot, OperationError> {
    service
        .reviews()?
        .snapshot(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn review_file(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ReviewFileRequest),
) -> Result<ReviewFileDiff, OperationError> {
    service
        .reviews()?
        .file(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}
