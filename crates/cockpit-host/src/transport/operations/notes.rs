use cockpit_core::CockpitService;
use cockpit_protocol::notes::{NotesRequest, NotesResponse};

use crate::transport::{Transport, error::OperationError};

pub async fn notes_execute(
    service: &CockpitService,
    _transport: Transport,
    request: NotesRequest,
) -> Result<NotesResponse, OperationError> {
    service.notes()?.execute(request).await.map_err(Into::into)
}
