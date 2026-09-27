use super::{inspection_error_response, requests::{decode_confirmation_request, decode_request}};
use cockpit_core::CockpitService;
use cockpit_protocol::{
    context::{ContextDirectory, ContextDocument},
    context_media::ContextMedia,
    library::{
        LibraryAddRequest, LibraryAttachmentRequest, LibraryConfluenceSpacesRequest, LibraryDirectoryRequest, LibraryDocumentRequest, LibraryListing,
        LibraryMediaRequest, LibraryOperation, LibraryRefreshRequest, LibraryRemoveRequest,
        LibraryReplaceRequest, LibraryResolution, LibraryResolveRequest, SpaceAddRequest,
        SpaceAttemptsDismissRequest, SpaceContextListing, SpaceContextRequest,
        SpaceUpdateRequest, SpaceUpdateScope, SpaceRemoveRequest,
    },
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;
const MAX_LIBRARY_REPORT_ROWS: usize = 256;

fn bounded_operation(mut operation: LibraryOperation) -> LibraryOperation {
    if let Some(report) = &mut operation.report
        && report.rows.len() > MAX_LIBRARY_REPORT_ROWS
    {
        report.rows.truncate(MAX_LIBRARY_REPORT_ROWS);
        report.truncated_rows = true;
    }
    operation
}

fn validate_space_request(
    target: &cockpit_protocol::library::SpaceTarget,
    items: usize,
) -> Result<(), ErrorResponse> {
    if target.session_id.is_empty()
        || target.session_id.len() > 96
        || !target
            .session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || target.space_id.is_empty()
        || target.space_id.len() > 128
        || !target
            .space_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
        || items > 5_000
    {
        return Err(super::stream_error(
            "invalid_library_request",
            "Expected a valid bounded Space request",
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn cockpit_library_listing(
    offset: Option<u32>,
    service: State<'_, CockpitService>,
) -> Result<LibraryListing, ErrorResponse> {
    service
        .library()
        .map_err(inspection_error_response)?
        .listing(offset)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_confluence_spaces(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<Vec<LibraryResolution>, ErrorResponse> {
    let request: LibraryConfluenceSpacesRequest =
        decode_request(request, "library Confluence spaces")?;
    if request.provider_id.is_empty()
        || request.provider_id.len() > 128
        || request.provider_id.chars().any(char::is_control)
    {
        return Err(super::stream_error(
            "invalid_library_request",
            "Expected a valid bounded Library request",
        ));
    }
    service
        .library()
        .map_err(inspection_error_response)?
        .confluence_spaces(&request.provider_id)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_resolve(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryResolution, ErrorResponse> {
    let request: LibraryResolveRequest = decode_request(request, "library resolve")?;
    service
        .library()
        .map_err(inspection_error_response)?
        .resolve(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_add(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let request: LibraryAddRequest = decode_request(request, "library add")?;
    if let Some(target) = &request.target {
        validate_space_request(target, 0)?;
    }
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .start_add(request)
        .await
        .map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_attachments(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let request: LibraryAttachmentRequest = decode_request(request, "library attachments")?;
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .start_attachments(request)
        .await
        .map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_refresh(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let request: LibraryRefreshRequest = decode_request(request, "library refresh")?;
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .start_refresh(request)
        .await
        .map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_operation(
    operation_id: String,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .operation(&operation_id)
        .await
        .map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_operation_cancel(
    operation_id: String,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .cancel(&operation_id)
        .await
        .map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_replace(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let request: LibraryReplaceRequest = decode_confirmation_request(request, "library replace")?;
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .start_replace(request)
        .await
        .map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_remove(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryListing, ErrorResponse> {
    let request: LibraryRemoveRequest = decode_confirmation_request(request, "library remove")?;
    service
        .library()
        .map_err(inspection_error_response)?
        .remove(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_directory(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextDirectory, ErrorResponse> {
    let request: LibraryDirectoryRequest = decode_request(request, "library directory")?;
    service
        .library()
        .map_err(inspection_error_response)?
        .directory(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_document(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextDocument, ErrorResponse> {
    let request: LibraryDocumentRequest = decode_request(request, "library document")?;
    service
        .library()
        .map_err(inspection_error_response)?
        .document(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_media(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<ContextMedia, ErrorResponse> {
    let request: LibraryMediaRequest = decode_request(request, "library media")?;
    service
        .library()
        .map_err(inspection_error_response)?
        .media(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_space_list(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<SpaceContextListing, ErrorResponse> {
    let request: SpaceContextRequest = decode_request(request, "library space list")?;
    validate_space_request(&request.target, 0)?;
    service
        .library()
        .map_err(inspection_error_response)?
        .space_listing(request.target)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_space_add(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let request: SpaceAddRequest = decode_request(request, "library space add")?;
    validate_space_request(
        &request.target,
        request.item_ids.len() + request.follow_ids.len(),
    )?;
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .start_space_add(request)
        .await
        .map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_space_attempts_dismiss(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<(), ErrorResponse> {
    let request: SpaceAttemptsDismissRequest =
        decode_request(request, "library space attempts dismiss")?;
    validate_space_request(
        &request.target,
        request.item_ids.len() + request.follow_ids.len(),
    )?;
    service
        .library()
        .map_err(inspection_error_response)?
        .dismiss_space_attempts(request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_library_space_update(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<LibraryOperation, ErrorResponse> {
    let request: SpaceUpdateRequest = decode_confirmation_request(request, "library space update")?;
    let count = match &request.scope {
        SpaceUpdateScope::Selection { item_ids, follow_ids } => item_ids.len() + follow_ids.len(),
        SpaceUpdateScope::All {} => 0,
    };
    validate_space_request(&request.target, count)?;
    let operation = service.library().map_err(inspection_error_response)?
        .start_space_update(request).await.map_err(inspection_error_response)?;
    Ok(bounded_operation(operation))
}

#[tauri::command]
pub async fn cockpit_library_space_remove(
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<SpaceContextListing, ErrorResponse> {
    let request: SpaceRemoveRequest = decode_confirmation_request(request, "library space remove")?;
    validate_space_request(&request.target, 0)?;
    service.library().map_err(inspection_error_response)?
        .space_remove(request).await.map_err(inspection_error_response)
}
