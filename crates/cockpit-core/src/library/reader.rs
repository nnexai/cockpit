use super::{LibraryService, store::Store};
use crate::{
    InspectionError,
    context::{AuthorizedRoot, enumerate_file_index, read_directory, read_document},
    context_media::read_media,
};
use cockpit_protocol::{
    context::{
        ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
        ContextFileIndex, ContextFileIndexSource, ContextFileIndexState,
    },
    context_media::{ContextMedia, ContextMediaRequest},
    library::{LibraryDirectoryRequest, LibraryDocumentRequest, LibraryFileIndexRequest, LibraryMediaRequest},
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
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = service.open()?;
            let _lock = store.shared()?;
            let authorized = service.authorized_root(&store)?;
            let request = ContextDirectoryRequest {
                binding_id: "library".into(),
                root_id: authorized.root_id().into(),
                path: request.path,
                offset: request.offset,
                revision: request.revision,
            };
            read_directory(&authorized, &request, &service.configuration.limits)
        })
        .await
        .map_err(|error| InspectionError::new("library_unavailable", error.to_string()))?
    }
    pub async fn document(
        &self,
        request: LibraryDocumentRequest,
    ) -> Result<ContextDocument, InspectionError> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = service.open()?;
            let _lock = store.shared()?;
            let authorized = service.authorized_root(&store)?;
            let request = ContextDocumentRequest {
                binding_id: "library".into(),
                root_id: authorized.root_id().into(),
                path: request.path,
                expected_revision: request.expected_revision,
                offset: request.offset,
            };
            read_document(&authorized, &request, &service.configuration.limits)
        })
        .await
        .map_err(|error| InspectionError::new("library_unavailable", error.to_string()))?
    }
    pub async fn media(
        &self,
        request: LibraryMediaRequest,
    ) -> Result<ContextMedia, InspectionError> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = service.open()?;
            let _lock = store.shared()?;
            let authorized = service.authorized_root(&store)?;
            let request = ContextMediaRequest {
                binding_id: "library".into(),
                root_id: authorized.root_id().into(),
                path: request.path,
                expected_revision: request.expected_revision,
            };
            read_media(
                authorized,
                &request,
                service.configuration.limits.context_preview_bytes as usize,
            )
        })
        .await
        .map_err(|error| InspectionError::new("library_unavailable", error.to_string()))?
    }
    pub async fn file_index(
        &self,
        request: LibraryFileIndexRequest,
    ) -> Result<ContextFileIndex, InspectionError> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let cache_root = std::path::PathBuf::from(&service.configuration.cache_root);
            let store = service.open()?;
            let _lock = store.shared()?;
            let authorized = service.authorized_root(&store)?;
            let root_id = authorized.root_id().to_owned();
            if request.mode == cockpit_protocol::library::LibraryFileIndexMode::Cached {
                let cached = crate::file_index_cache::load(authorized.canonical_path(), authorized.root_kind(), &cache_root);
                return Ok(match cached {
                    Some(cached) => ContextFileIndex {
                        binding_id: "library".into(),
                        root_id,
                        files: cached.files,
                        truncated: cached.truncated,
                        source: cached.source,
                        state: ContextFileIndexState::Cached,
                        diagnostics: Vec::new(),
                    },
                    None => ContextFileIndex {
                        binding_id: "library".into(),
                        root_id,
                        files: Vec::new(),
                        truncated: false,
                        source: ContextFileIndexSource::Walk,
                        state: ContextFileIndexState::Miss,
                        diagnostics: Vec::new(),
                    },
                });
            }
            let _index = store.index_shared()?;
            authorized.revalidate()?;
            let result = enumerate_file_index(&authorized, false, None)?;
            crate::file_index_cache::store(
                authorized.canonical_path(),
                authorized.root_kind(),
                ContextFileIndexSource::Walk,
                result.1,
                result.0.clone(),
                &cache_root,
            );
            Ok(ContextFileIndex {
                binding_id: "library".into(),
                root_id,
                files: result.0,
                truncated: result.1,
                source: ContextFileIndexSource::Walk,
                state: ContextFileIndexState::Fresh,
                diagnostics: Vec::new(),
            })
        })
        .await
        .map_err(|error| InspectionError::new("library_unavailable", error.to_string()))?
}
}
