use axum::extract::{Query, rejection::QueryRejection};
use cockpit_core::CockpitService;
use cockpit_protocol::{
    context::{ContextDirectory, ContextDocument, ContextFileIndex},
    context_media::ContextMedia,
    library::{
        LibraryAddRequest, LibraryAttachmentRequest, LibraryConfluenceSpacesRequest,
        LibraryDirectoryRequest, LibraryDocumentRequest, LibraryFileIndexRequest, LibraryListing,
        LibraryMediaRequest, LibraryOperation, LibraryRefreshRequest, LibraryRemoveRequest,
        LibraryReplaceRequest, LibraryResolution, LibraryResolveRequest, SpaceAddRequest,
        SpaceContextListing, SpaceContextRequest, SpaceRemoveRequest, SpaceRepositoriesRequest,
        SpaceTarget,
    },
};
use serde::Deserialize;

use super::super::{
    Transport,
    error::OperationError,
    guard::{valid_resource_id, valid_session_id},
    http_input::{self, Reject},
    limits::{LIBRARY_PAGE_ITEMS, bounded_operation},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingQuery {
    pub offset: Option<u32>,
}

pub fn listing_input()
-> impl FnOnce((Result<Query<ListingQuery>, QueryRejection>,)) -> Result<Option<u32>, OperationError>
{
    |args| {
        http_input::mapped_query(Reject(
            "invalid_library_request",
            "Expected a valid bounded Library request",
        ))(args)
        .map(|query: ListingQuery| query.offset)
    }
}

fn invalid_library_request() -> OperationError {
    OperationError::rejected(
        "invalid_library_request",
        "Expected a valid bounded Library request",
    )
}

fn valid_target(target: &SpaceTarget) -> bool {
    valid_session_id(&target.session_id) && valid_resource_id(&target.space_id)
}

fn validate_target(
    transport: Transport,
    target: &SpaceTarget,
    items: usize,
) -> Result<(), OperationError> {
    if valid_target(target) && items <= LIBRARY_PAGE_ITEMS {
        return Ok(());
    }
    Err(match transport {
        Transport::Gateway => invalid_library_request(),
        Transport::Native => OperationError::rejected(
            "invalid_library_request",
            "Expected a valid bounded Space request",
        ),
    })
}

fn invalid_path(path: &str) -> bool {
    path.starts_with('/') || path.split('/').any(|segment| segment == "..")
}

fn validate_path(transport: Transport, path: &str) -> Result<(), OperationError> {
    if transport == Transport::Gateway && invalid_path(path) {
        return Err(invalid_library_request());
    }
    Ok(())
}

fn validate_attachments(
    transport: Transport,
    request: &LibraryAttachmentRequest,
) -> Result<(), OperationError> {
    if transport == Transport::Gateway
        && (request.item_id.is_empty()
            || request.item_id.len() > 512
            || request.item_id.chars().any(char::is_control)
            || request.attachment_ids.is_empty()
            || request.attachment_ids.len() > 256
            || request
                .attachment_ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control)))
    {
        return Err(invalid_library_request());
    }
    Ok(())
}

fn validate_repositories(
    transport: Transport,
    request: &SpaceRepositoriesRequest,
) -> Result<(), OperationError> {
    match transport {
        Transport::Gateway => {
            if !valid_target(&request.target) || request.repository_paths.len() > 64 {
                return Err(invalid_library_request());
            }
        }
        Transport::Native => {
            // The 5,000-item Space bound precedes the distinct 64-repository rejection.
            validate_target(transport, &request.target, request.repository_paths.len())?;
            if request.repository_paths.len() > 64 {
                return Err(OperationError::rejected(
                    "invalid_library_request",
                    "Too many selected repositories",
                ));
            }
        }
    }
    Ok(())
}

pub async fn library_listing(
    service: &CockpitService,
    _: Transport,
    offset: Option<u32>,
) -> Result<LibraryListing, OperationError> {
    Ok(service.library()?.listing(offset).await?)
}

pub async fn library_resolve(
    service: &CockpitService,
    _: Transport,
    request: LibraryResolveRequest,
) -> Result<LibraryResolution, OperationError> {
    Ok(service.library()?.resolve(request).await?)
}

pub async fn library_confluence_spaces(
    service: &CockpitService,
    _: Transport,
    request: LibraryConfluenceSpacesRequest,
) -> Result<Vec<LibraryResolution>, OperationError> {
    if request.provider_id.is_empty()
        || request.provider_id.len() > 128
        || request.provider_id.chars().any(char::is_control)
    {
        return Err(invalid_library_request());
    }
    Ok(service
        .library()?
        .confluence_spaces(&request.provider_id)
        .await?)
}

pub async fn library_add(
    service: &CockpitService,
    transport: Transport,
    request: LibraryAddRequest,
) -> Result<LibraryOperation, OperationError> {
    if let Some(target) = &request.target {
        validate_target(transport, target, 0)?;
    }
    Ok(bounded_operation(
        service.library()?.start_add(request).await?,
    ))
}

pub async fn library_attachments(
    service: &CockpitService,
    transport: Transport,
    request: LibraryAttachmentRequest,
) -> Result<LibraryOperation, OperationError> {
    validate_attachments(transport, &request)?;
    Ok(bounded_operation(
        service.library()?.start_attachments(request).await?,
    ))
}

pub async fn library_refresh(
    service: &CockpitService,
    transport: Transport,
    request: LibraryRefreshRequest,
) -> Result<LibraryOperation, OperationError> {
    if transport == Transport::Gateway
        && matches!(&request, LibraryRefreshRequest::Items { item_ids } if item_ids.len() > LIBRARY_PAGE_ITEMS)
    {
        return Err(invalid_library_request());
    }
    Ok(bounded_operation(
        service.library()?.start_refresh(request).await?,
    ))
}

pub async fn library_operation(
    service: &CockpitService,
    _: Transport,
    operation_id: String,
) -> Result<LibraryOperation, OperationError> {
    Ok(bounded_operation(
        service.library()?.operation(&operation_id).await?,
    ))
}

pub async fn library_operation_cancel(
    service: &CockpitService,
    _: Transport,
    operation_id: String,
) -> Result<LibraryOperation, OperationError> {
    Ok(bounded_operation(
        service.library()?.cancel(&operation_id).await?,
    ))
}

pub async fn library_replace(
    service: &CockpitService,
    transport: Transport,
    request: LibraryReplaceRequest,
) -> Result<LibraryOperation, OperationError> {
    if transport == Transport::Gateway && request.confirmed.len() > LIBRARY_PAGE_ITEMS {
        return Err(invalid_library_request());
    }
    Ok(bounded_operation(
        service.library()?.start_replace(request).await?,
    ))
}

pub async fn library_remove(
    service: &CockpitService,
    _: Transport,
    request: LibraryRemoveRequest,
) -> Result<LibraryListing, OperationError> {
    Ok(service.library()?.remove(request).await?)
}

pub async fn library_directory(
    service: &CockpitService,
    transport: Transport,
    request: LibraryDirectoryRequest,
) -> Result<ContextDirectory, OperationError> {
    validate_path(transport, &request.path)?;
    Ok(service.library()?.directory(request).await?)
}

pub async fn library_file_index(
    service: &CockpitService,
    _: Transport,
    request: LibraryFileIndexRequest,
) -> Result<ContextFileIndex, OperationError> {
    Ok(service.library()?.file_index(request).await?)
}

pub async fn library_document(
    service: &CockpitService,
    transport: Transport,
    request: LibraryDocumentRequest,
) -> Result<ContextDocument, OperationError> {
    validate_path(transport, &request.path)?;
    Ok(service.library()?.document(request).await?)
}

pub async fn library_media(
    service: &CockpitService,
    transport: Transport,
    request: LibraryMediaRequest,
) -> Result<ContextMedia, OperationError> {
    validate_path(transport, &request.path)?;
    Ok(service.library()?.media(request).await?)
}

pub async fn library_space_list(
    service: &CockpitService,
    transport: Transport,
    request: SpaceContextRequest,
) -> Result<SpaceContextListing, OperationError> {
    validate_target(transport, &request.target, 0)?;
    Ok(service.library()?.space_listing(&request.target).await?)
}

pub async fn library_space_add(
    service: &CockpitService,
    transport: Transport,
    request: SpaceAddRequest,
) -> Result<LibraryOperation, OperationError> {
    validate_target(transport, &request.target, request.item_ids.len())?;
    Ok(bounded_operation(
        service.library()?.start_space_add(request).await?,
    ))
}

pub async fn library_space_repositories(
    service: &CockpitService,
    transport: Transport,
    request: SpaceRepositoriesRequest,
) -> Result<SpaceContextListing, OperationError> {
    validate_repositories(transport, &request)?;
    Ok(service.library()?.space_repositories(request).await?)
}

pub async fn library_space_remove(
    service: &CockpitService,
    transport: Transport,
    request: SpaceRemoveRequest,
) -> Result<SpaceContextListing, OperationError> {
    validate_target(transport, &request.target, request.item_ids.len())?;
    Ok(service.library()?.space_remove(request).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::v1::ErrorResponse;

    fn target() -> SpaceTarget {
        SpaceTarget {
            session_id: "session-1".into(),
            space_id: "space:1".into(),
        }
    }

    fn code(error: OperationError) -> String {
        ErrorResponse::from(error).code
    }

    #[test]
    fn space_validation_enforces_page_and_target_boundaries() {
        for transport in [Transport::Gateway, Transport::Native] {
            assert!(validate_target(transport, &target(), LIBRARY_PAGE_ITEMS).is_ok());
        }
        for transport in [Transport::Gateway, Transport::Native] {
            assert_eq!(
                code(validate_target(transport, &target(), 5_001).unwrap_err()),
                "invalid_library_request"
            );
        }
        let mut invalid = target();
        invalid.space_id = "space/1".into();
        assert!(validate_target(Transport::Gateway, &invalid, 0).is_err());
        assert!(validate_target(Transport::Native, &invalid, 0).is_err());
    }

    #[test]
    fn paths_remain_gateway_only_and_do_not_reject_dot_names() {
        for path in ["/absolute", "../parent", "nested/../parent"] {
            assert!(validate_path(Transport::Gateway, path).is_err());
            assert!(validate_path(Transport::Native, path).is_ok());
        }
        for path in ["", "relative/file", "..name", "nested/.../file"] {
            assert!(validate_path(Transport::Gateway, path).is_ok());
        }
    }

    #[test]
    fn repository_count_enforces_bounds_and_invalid_targets() {
        let mut request = SpaceRepositoriesRequest {
            target: target(),
            repository_paths: vec!["repository".into(); 65],
        };
        for transport in [Transport::Gateway, Transport::Native] {
            assert_eq!(
                code(validate_repositories(transport, &request).unwrap_err()),
                "invalid_library_request"
            );
        }
        request.repository_paths.resize(5_001, "repository".into());
        assert_eq!(
            code(validate_repositories(Transport::Native, &request).unwrap_err()),
            "invalid_library_request"
        );
        request.target.space_id = "invalid/space".into();
        for transport in [Transport::Gateway, Transport::Native] {
            assert_eq!(
                code(validate_repositories(transport, &request).unwrap_err()),
                "invalid_library_request"
            );
        }
        request.target = target();
        request.repository_paths.truncate(64);
        assert!(validate_repositories(Transport::Native, &request).is_ok());
        assert!(validate_repositories(Transport::Gateway, &request).is_ok());
    }

    #[test]
    fn listing_query_rejects_unknown_fields() {
        assert!(serde_json::from_str::<ListingQuery>(r#"{"offset":7}"#).is_ok());
        assert!(serde_json::from_str::<ListingQuery>(r#"{"offset":7,"extra":true}"#).is_err());
    }

    #[test]
    fn attachment_validation_remains_gateway_only_at_each_bound() {
        let mut request = LibraryAttachmentRequest {
            item_id: "item".into(),
            attachment_ids: vec!["attachment".into(); 256],
            action: cockpit_protocol::library::LibraryAttachmentAction::Download,
        };
        assert!(validate_attachments(Transport::Gateway, &request).is_ok());
        request.attachment_ids.push("attachment".into());
        assert!(validate_attachments(Transport::Gateway, &request).is_err());
        assert!(validate_attachments(Transport::Native, &request).is_ok());
        request.attachment_ids.clear();
        assert!(validate_attachments(Transport::Gateway, &request).is_err());
        request.attachment_ids.push("a".repeat(512));
        request.item_id = "i".repeat(512);
        assert!(validate_attachments(Transport::Gateway, &request).is_ok());
        request.item_id.push('i');
        assert!(validate_attachments(Transport::Gateway, &request).is_err());
        request.item_id = "item".into();
        request.attachment_ids[0].push('a');
        assert!(validate_attachments(Transport::Gateway, &request).is_err());
        request.attachment_ids[0] = "control\n".into();
        assert!(validate_attachments(Transport::Gateway, &request).is_err());
        assert!(validate_attachments(Transport::Native, &request).is_ok());
    }
}
