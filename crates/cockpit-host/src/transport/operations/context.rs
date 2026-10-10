use cockpit_core::{CockpitService, context_search::ContextSearchService};
use cockpit_protocol::{
    context::{
        ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
        ContextFileIndex, ContextFileIndexRequest,
    },
    context_media::{ContextMedia, ContextMediaRequest},
    context_search::{
        ContextInvalidationRequest, ContextInvalidationResponse, ContextSearchRequest,
        ContextSearchResponse,
    },
};

use crate::transport::{Transport, error::OperationError};

pub async fn context_directory(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ContextDirectoryRequest),
) -> Result<ContextDirectory, OperationError> {
    service
        .contexts()?
        .directory(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn context_file_index(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ContextFileIndexRequest),
) -> Result<ContextFileIndex, OperationError> {
    service
        .contexts()?
        .file_index(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn context_document(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ContextDocumentRequest),
) -> Result<ContextDocument, OperationError> {
    service
        .contexts()?
        .document(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn context_search(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ContextSearchRequest),
) -> Result<ContextSearchResponse, OperationError> {
    let contexts = service.contexts()?.clone();
    ContextSearchService::new(contexts)
        .search(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn context_invalidate(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ContextInvalidationRequest),
) -> Result<ContextInvalidationResponse, OperationError> {
    let contexts = service.contexts()?.clone();
    ContextSearchService::new(contexts)
        .invalidate(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn context_media(
    service: &CockpitService,
    _transport: Transport,
    (session_id, viewer_id, request): (String, String, ContextMediaRequest),
) -> Result<ContextMedia, OperationError> {
    service
        .contexts()?
        .media(&session_id, &viewer_id, &request)
        .await
        .map_err(Into::into)
}
