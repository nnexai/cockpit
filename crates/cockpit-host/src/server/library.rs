use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cockpit_core::CockpitService;
use cockpit_protocol::library::{
    LibraryAddRequest, LibraryConfluenceSpacesRequest, LibraryDirectoryRequest, LibraryDocumentRequest, LibraryFileIndexRequest, LibraryMediaRequest,
    LibraryOperation, LibraryRefreshRequest, LibraryRemoveRequest, LibraryReplaceRequest,
    LibraryResolveRequest, SpaceAddRequest, SpaceContextRequest,
    SpaceRepositoriesRequest, SpaceRemoveRequest,
};
use serde::Deserialize;

use super::{
    bad_request, inspection_error, require_origin, valid_resource_id, valid_session_id,
};

// 512 confirmations with 4 KiB paths, JSON escaping, hashes, and request metadata.
const MAX_LIBRARY_CONFIRMATION_REQUEST_BYTES: usize = 13 * 1024 * 1024;
const MAX_LIBRARY_PAGE_ITEMS: usize = 5_000;
const MAX_LIBRARY_REPORT_ROWS: usize = 256;

pub(super) fn routes() -> Router<CockpitService> {
    Router::new()
        .route("/api/v1/library", get(listing))
        .route("/api/v1/library/resolve", post(resolve))
        .route("/api/v1/library/add", post(add))
        .route("/api/v1/library/refresh", post(refresh))
        .route("/api/v1/library/attachments", post(attachments))
        .route("/api/v1/library/operations/{id}", get(operation))
        .route("/api/v1/library/operations/{id}/cancel", post(cancel))
        .route("/api/v1/library/replace", post(replace))
        .route("/api/v1/library/remove", post(remove))
        .route("/api/v1/library/files", post(files))
        .route("/api/v1/library/directory", post(directory))
        .route("/api/v1/library/document", post(document))
        .route("/api/v1/library/media", post(media))
        .route("/api/v1/library/confluence/spaces", post(confluence_spaces))
        .route("/api/v1/library/space/list", post(space_list))
        .route("/api/v1/library/space/add", post(space_add))
        .route("/api/v1/library/space/remove", post(space_remove))
        .route("/api/v1/library/space/repositories", post(space_repositories))
        .layer(DefaultBodyLimit::max(MAX_LIBRARY_CONFIRMATION_REQUEST_BYTES))
        .route_layer(middleware::from_fn(require_origin))
}

async fn attachments(
    State(service): State<CockpitService>,
    body: Result<Json<cockpit_protocol::library::LibraryAttachmentRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.item_id.is_empty()
        || request.item_id.len() > 512
        || request.item_id.chars().any(char::is_control)
        || request.attachment_ids.is_empty()
        || request.attachment_ids.len() > 256
        || request.attachment_ids.iter().any(|id| {
            id.is_empty() || id.len() > 512 || id.chars().any(char::is_control)
        })
    {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.start_attachments(request).await {
            Ok(value) => operation_response(value),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListingQuery {
    offset: Option<u32>,
}

fn request<T: serde::de::DeserializeOwned>(
    body: Result<Json<T>, JsonRejection>,
) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| invalid_request())
}

fn invalid_request() -> Response {
    bad_request(
        "invalid_library_request",
        "Expected a valid bounded Library request",
    )
}

fn valid_target(target: &cockpit_protocol::library::SpaceTarget) -> bool {
    valid_session_id(&target.session_id) && valid_resource_id(&target.space_id)
}

fn invalid_path(path: &str) -> bool {
    path.starts_with('/') || path.split('/').any(|segment| segment == "..")
}

fn operation_response(mut operation: LibraryOperation) -> Response {
    if let Some(report) = &mut operation.report
        && report.rows.len() > MAX_LIBRARY_REPORT_ROWS
    {
        report.rows.truncate(MAX_LIBRARY_REPORT_ROWS);
        report.truncated_rows = true;
    }
    Json(operation).into_response()
}

async fn listing(
    State(service): State<CockpitService>,
    query: Result<Query<ListingQuery>, QueryRejection>,
) -> Response {
    let Query(query) = match query {
        Ok(query) => query,
        Err(_) => return invalid_request(),
    };
    match service.library() {
        Ok(library) => match library.listing(query.offset).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn resolve(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryResolveRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    match service.library() {
        Ok(library) => match library.resolve(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn confluence_spaces(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryConfluenceSpacesRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.provider_id.is_empty()
        || request.provider_id.len() > 128
        || request.provider_id.chars().any(char::is_control)
    {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.confluence_spaces(&request.provider_id).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn add(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryAddRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request
        .target
        .as_ref()
        .is_some_and(|target| !valid_target(target))
    {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.start_add(request).await {
            Ok(value) => operation_response(value),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn refresh(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryRefreshRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if matches!(&request, LibraryRefreshRequest::Items { item_ids } if item_ids.len() > MAX_LIBRARY_PAGE_ITEMS)
    {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.start_refresh(request).await {
            Ok(value) => operation_response(value),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn operation(State(service): State<CockpitService>, Path(id): Path<String>) -> Response {
    match service.library() {
        Ok(library) => match library.operation(&id).await {
            Ok(value) => operation_response(value),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn cancel(State(service): State<CockpitService>, Path(id): Path<String>) -> Response {
    match service.library() {
        Ok(library) => match library.cancel(&id).await {
            Ok(value) => operation_response(value),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn replace(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryReplaceRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.confirmed.len() > MAX_LIBRARY_PAGE_ITEMS {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.start_replace(request).await {
            Ok(value) => operation_response(value),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn remove(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryRemoveRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    match service.library() {
        Ok(library) => match library.remove(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn directory(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryDirectoryRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if invalid_path(&request.path) {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.directory(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn files(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryFileIndexRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    match service.library() {
        Ok(library) => match library.file_index(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn document(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryDocumentRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if invalid_path(&request.path) {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.document(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn media(
    State(service): State<CockpitService>,
    body: Result<Json<LibraryMediaRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if invalid_path(&request.path) {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.media(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn space_list(
    State(service): State<CockpitService>,
    body: Result<Json<SpaceContextRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if !valid_target(&request.target) {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.space_listing(&request.target).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn space_add(
    State(service): State<CockpitService>,
    body: Result<Json<SpaceAddRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if !valid_target(&request.target)
        || request.item_ids.len() > MAX_LIBRARY_PAGE_ITEMS
    {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.start_space_add(request).await {
            Ok(value) => operation_response(value),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn space_repositories(
    State(service): State<CockpitService>,
    body: Result<Json<SpaceRepositoriesRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if !valid_target(&request.target) || request.repository_paths.len() > 64 {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.space_repositories(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}

async fn space_remove(
    State(service): State<CockpitService>,
    body: Result<Json<SpaceRemoveRequest>, JsonRejection>,
) -> Response {
    let request = match request(body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if !valid_target(&request.target) || request.item_ids.len() > MAX_LIBRARY_PAGE_ITEMS {
        return invalid_request();
    }
    match service.library() {
        Ok(library) => match library.space_remove(request).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => inspection_error(error),
        },
        Err(error) => inspection_error(error),
    }
}
