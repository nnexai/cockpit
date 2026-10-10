use cockpit_core::CockpitService;
use cockpit_protocol::viewer::{ViewerContext, ViewerOpenRequest, ViewerSourceOptions};

use crate::transport::{Transport, error::OperationError};

pub async fn viewer_sources(
    service: &CockpitService,
    _transport: Transport,
    (session_id, pane_id): (String, String),
) -> Result<ViewerSourceOptions, OperationError> {
    service
        .viewers()?
        .sources(&session_id, &pane_id)
        .await
        .map_err(Into::into)
}

pub async fn viewer_open(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, ViewerOpenRequest),
) -> Result<ViewerContext, OperationError> {
    service
        .viewers()?
        .open(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn viewer_release(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id): (String, String),
) -> Result<(), OperationError> {
    service
        .viewers()?
        .release(&session_id, &viewer_id)
        .await
        .map_err(Into::into)
}
