use super::{inspection_error_response, requests::decode_request};
use cockpit_core::CockpitService;
use cockpit_protocol::{
    context::{ContextDirectory, ContextDocument},
    context_media::ContextMedia,
    library::{
        LibraryAddRequest, LibraryDirectoryRequest, LibraryDocumentRequest, LibraryListing,
        LibraryMediaRequest, LibraryOperation, LibraryRefreshRequest, LibraryRemoveRequest,
        LibraryReplaceRequest, LibraryResolution, LibraryResolveRequest,
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
    let operation = service
        .library()
        .map_err(inspection_error_response)?
        .start_add(request)
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
    let request: LibraryReplaceRequest = decode_request(request, "library replace")?;
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
    let request: LibraryRemoveRequest = decode_request(request, "library remove")?;
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
