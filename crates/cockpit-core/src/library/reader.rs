use super::{LibraryService, store::Store};
use crate::{
    InspectionError,
    context::{AuthorizedRoot, read_directory, read_document},
    context_media::read_media,
};
use cockpit_protocol::{
    context::{ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest},
    context_media::{ContextMedia, ContextMediaRequest},
    library::{LibraryDirectoryRequest, LibraryDocumentRequest, LibraryMediaRequest},
};

impl LibraryService {
    pub(super) fn authorized_root(&self, store: &Store) -> Result<AuthorizedRoot, InspectionError> {
        let dir = store
            .root
            .try_clone()
            .map_err(|e| InspectionError::new("library_unavailable", e.to_string()))?;
        AuthorizedRoot::library(
            store.path.clone(),
            dir,
            self.configuration.limits.context_tree_depth,
        )
    }
    pub async fn directory(
        &self,
        request: LibraryDirectoryRequest,
    ) -> Result<ContextDirectory, InspectionError> {
        let store = self.open()?;
        let _lock = store.shared()?;
        let authorized = self.authorized_root(&store)?;
        let request = ContextDirectoryRequest {
            binding_id: "library".into(),
            root_id: authorized.root_id().into(),
            path: request.path,
            offset: request.offset,
            revision: request.revision,
        };
        read_directory(&authorized, &request, &self.configuration.limits)
    }
    pub async fn document(
        &self,
        request: LibraryDocumentRequest,
    ) -> Result<ContextDocument, InspectionError> {
        let store = self.open()?;
        // Keep the shared lock through the bounded read and its existing revision
        // recheck, not just openat: Context's reader re-stats the original path.
        let _lock = store.shared()?;
        let authorized = self.authorized_root(&store)?;
        let request = ContextDocumentRequest {
            binding_id: "library".into(),
            root_id: authorized.root_id().into(),
            path: request.path,
            expected_revision: request.expected_revision,
            offset: request.offset,
        };
        read_document(&authorized, &request, &self.configuration.limits)
    }
    pub async fn media(
        &self,
        request: LibraryMediaRequest,
    ) -> Result<ContextMedia, InspectionError> {
        let store = self.open()?;
        let _lock = store.shared()?;
        let authorized = self.authorized_root(&store)?;
        let request = ContextMediaRequest {
            binding_id: "library".into(),
            root_id: authorized.root_id().into(),
            path: request.path,
            expected_revision: request.expected_revision,
        };
        read_media(
            authorized,
            &request,
            self.configuration.limits.context_preview_bytes as usize,
        )
    }
}
