use std::path::{Path, PathBuf};

use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, DirBuilder, DirBuilderExt, MetadataExt};
use cockpit_protocol::library::SpaceTarget;
use cockpit_protocol::orchestration::{OwnedAttemptDirectoryIdentity, OwnedResourceIdentity};
use cockpit_protocol::projects::{OwnedAttemptDirectoryRequest, OwnedCheckoutRequest, WorkspaceCheckoutOwnership,
    WorkspaceOperation, WorkspaceOperationState, WorkspaceOperationStep, WorkspaceOwnedResource,
    WorkspaceSetupMode, WorkspaceSetupPlan};

use crate::{InspectionError, library::LibraryService,
    project_store::{OwnedCheckoutJournal, OwnedCreationReceipt, open_dir_nofollow_absolute}};
use super::{ProjectService, error_response, validate_session, validate_text};

impl ProjectService {
    /// Create one private, empty, per-run output directory; never copy or own its original project.
    pub async fn prepare_owned_attempt_directory(&self, session: &str, request: &OwnedAttemptDirectoryRequest,
        library: &LibraryService) -> Result<WorkspaceOperation, InspectionError>
    {
        validate_session(session)?;
        for id in [&request.operation_id, &request.run_id] {
            uuid::Uuid::parse_str(id).map_err(|_| conflict("attempt and operation IDs must be UUIDs"))?;
        }
        validate_text(&request.source_workspace_id, "source_workspace_id", 256)?;
        validate_text(&request.source_root, "source_root", 4096)?;
        let source = open_dir_nofollow_absolute(Path::new(&request.source_root)).map_err(io_error)?;
        let canonical_source = std::fs::canonicalize(&request.source_root).map_err(io_error)?;
        if canonical_source != Path::new(&request.source_root) || !source.dir_metadata().map_err(io_error)?.is_dir() {
            return Err(conflict("original Directory project must have an exact no-follow root"));
        }
        let _lease = self.store.acquire_execution_lease(&request.operation_id)?;
        let prior = self.store.list()?.into_iter().find(|o| o.operation_id == request.operation_id);
        match prior {
            Some(operation) => {
                let journal = self.store.owned_checkout(&request.operation_id)?.ok_or_else(|| conflict("attempt operation has no directory journal"))?;
                if journal.attempt_directory.as_ref() != Some(request) || operation.session_id != session
                    || operation.plan.ownership != WorkspaceCheckoutOwnership::OwnedAttemptDirectory || operation.cancel_requested
                    || self.adapter.project_endpoint_identity(session).await? != operation.plan.endpoint_identity
                { return Err(conflict("attempt request differs from its retained operation")); }
                if let Some(receipt) = self.store.read_owned_creation(&request.operation_id)? {
                    crate::project_teardown::validate_creation_receipt(&receipt, &operation)?;
                    let identity = receipt.resources.iter().find_map(|r| match r {
                        OwnedResourceIdentity::AttemptDirectory { identity } => Some(identity), _ => None,
                    }).ok_or_else(|| conflict("attempt directory creation proof is missing"))?;
                    self.validate_attempt_directory_identity(identity)?;
                    if operation.workspace_id.is_some() || operation.step == WorkspaceOperationStep::CheckoutPrepared { return Ok(operation); }
                    return self.store.update(&request.operation_id, Some(operation.generation), |o| {
                        o.step = WorkspaceOperationStep::CheckoutPrepared; o.state = WorkspaceOperationState::Running;
                        o.error = None; o.resume_allowed = false; Ok(())
                    });
                }
                if journal.directory_requested {
                    return Err(unknown("interrupted exclusive directory creation has no physical receipt; the existing path is protected, never adopted"));
                }
            }
            None => {
                let checkout_path = std::fs::canonicalize(&self.configuration.state_root).map_err(io_error)?
                    .join("orchestration/attempts").join(&request.run_id);
                if checkout_path.starts_with(&canonical_source) || canonical_source.starts_with(&checkout_path) {
                    return Err(conflict("attempt directory overlaps the original source project"));
                }
                let (parent_path, _) = self.attempts_parent(true)?;
                if parent_path.join(&request.run_id) != checkout_path {
                    return Err(conflict("attempt parent routing changed before preparation"));
                }
                let endpoint = self.adapter.project_endpoint_identity(session).await?;
                let snapshot = self.adapter.session_snapshot(session).await?;
                if snapshot.session_id != session || snapshot.spaces.iter().filter(|s| s.id == request.source_workspace_id).count() != 1
                    || self.adapter.project_endpoint_identity(session).await? != endpoint
                { return Err(conflict("attempt preparation requires an exact source Space and endpoint")); }
                let context = library.space_listing(&SpaceTarget {
                    session_id: session.into(), space_id: request.source_workspace_id.clone(),
                }).await?;
                if self.adapter.project_endpoint_identity(session).await? != endpoint {
                    return Err(conflict("source Library context endpoint changed"));
                }
                let checkout_path = checkout_path.to_str().ok_or_else(|| conflict("attempt path is not UTF-8"))?.to_owned();
                let journal = OwnedCheckoutJournal {
                    request: OwnedCheckoutRequest { operation_id: request.operation_id.clone(), repository_checkout: request.source_root.clone(),
                        checkout_path: checkout_path.clone(), branch: String::new(), base: String::new(), artifact: None,
                        source_workspace_id: request.source_workspace_id.clone() },
                    borrowed_directory: false, attempt_directory: Some(request.clone()),
                    directory_requested: false,
                    resume_binding: None,
                    existing_workspace_ids: snapshot.spaces.into_iter().map(|s| s.id).collect(),
                    existing_terminal_ids: snapshot.panes.into_iter().map(|p| p.terminal_id).collect(),
                    context_item_ids: context.items.into_iter().map(|i| i.item_id).collect(),
                    context_repository_paths: context.repository_paths, git_requested: false, bind: None,
                };
                self.store.persist_owned_checkout_plan(WorkspaceSetupPlan {
                    operation_id: request.operation_id.clone(), generation: 1, endpoint_identity: endpoint,
                    session_id: session.into(), repository: None, mode: WorkspaceSetupMode::Open,
                    ownership: WorkspaceCheckoutOwnership::OwnedAttemptDirectory, branch: None, base: None,
                    checkout_path, label: request.run_id.clone(), focus: false, artifact: None, linked_artifacts: vec![],
                    effects: vec!["create private isolated attempt output directory".into()], warnings: vec![],
                }, journal)?;
            }
        };
        let operation = self.store.load(&request.operation_id)?;
        let (parent_path, parent) = self.attempts_parent(true)?;
        if parent_path.join(&request.run_id) != Path::new(&operation.plan.checkout_path) {
            return Err(conflict("attempt directory routing changed"));
        }
        // Persist intent before the exclusive mkdir. EEXIST never grants this operation ownership.
        self.store.update_owned_checkout(&request.operation_id, Some(operation.generation), |o, retained| {
            if o.cancel_requested { return Err(conflict("attempt preparation was cancelled")); }
            retained.as_mut().ok_or_else(|| conflict("attempt journal disappeared"))?.directory_requested = true;
            o.state = WorkspaceOperationState::OutcomeUnknown; o.resume_allowed = false;
            o.error = Some(error_response("owned_attempt_directory_pending", "awaiting exclusive directory creation outcome")); Ok(())
        })?;
        let mut builder = DirBuilder::new(); builder.mode(0o700);
        parent.create_dir_with(&request.run_id, &builder).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                unknown("attempt directory already exists without this creation receipt; it is protected")
            } else { io_error(error) }
        })?;
        let directory = parent.open_dir_nofollow(&request.run_id).map_err(io_error)?;
        private_directory(&directory)?;
        parent.open(".").and_then(|file| file.sync_all()).map_err(io_error)?;
        let metadata = directory.dir_metadata().map_err(io_error)?;
        let identity = OwnedAttemptDirectoryIdentity {
            root: operation.plan.checkout_path.clone(), device: metadata.dev().to_string(), inode: metadata.ino().to_string(),
            creation_operation_id: request.operation_id.clone(),
        };
        // Confirm path routing still reaches the pinned creation, without putting metadata in its output directory.
        self.validate_attempt_directory_identity(&identity)?;
        self.store.update(&request.operation_id, None, |o| {
            o.owned_resources.push(WorkspaceOwnedResource { kind: "attempt_directory".into(),
                path: identity.root.clone(), created_by_operation: true }); Ok(())
        })?;
        self.store.write_owned_creation(&OwnedCreationReceipt {
            operation_id: request.operation_id.clone(), resources: vec![OwnedResourceIdentity::AttemptDirectory { identity }],
            git_capture_error: None, moved_space: None,
        })?;
        self.store.update(&request.operation_id, None, |o| {
            o.step = WorkspaceOperationStep::CheckoutPrepared; o.state = WorkspaceOperationState::Running;
            o.error = None; o.resume_allowed = false; Ok(())
        })
    }

    pub(super) fn validate_attempt_directory_identity(&self, identity: &OwnedAttemptDirectoryIdentity) -> Result<(), InspectionError> {
        let journal = self.store.owned_checkout(&identity.creation_operation_id)?.ok_or_else(|| conflict("attempt directory has no operation journal"))?;
        let request = journal.attempt_directory.ok_or_else(|| conflict("operation did not create an attempt directory"))?;
        if !journal.directory_requested || request.operation_id != identity.creation_operation_id
            || uuid::Uuid::parse_str(&identity.creation_operation_id).is_err()
        { return Err(conflict("attempt directory differs from its exact creation operation")); }
        let (parent_path, parent) = self.attempts_parent(false)?;
        if parent_path.join(&request.run_id) != Path::new(&identity.root) {
            return Err(conflict("attempt directory does not equal its private per-run path"));
        }
        let directory = parent.open_dir_nofollow(&request.run_id).map_err(io_error)?;
        private_directory(&directory)?;
        let metadata = directory.dir_metadata().map_err(io_error)?;
        if metadata.dev().to_string() != identity.device || metadata.ino().to_string() != identity.inode {
            return Err(conflict("attempt directory physical identity changed"));
        }
        Ok(())
    }

    fn attempts_parent(&self, create: bool) -> Result<(PathBuf, Dir), InspectionError> {
        let root_path = std::fs::canonicalize(&self.configuration.state_root).map_err(io_error)?;
        let fresh = open_dir_nofollow_absolute(&root_path).map_err(io_error)?.dir_metadata().map_err(io_error)?;
        let retained = self.store.state_dir().dir_metadata().map_err(io_error)?;
        if fresh.dev() != retained.dev() || fresh.ino() != retained.ino() {
            return Err(conflict("state root physical identity changed"));
        }
        let root = self.store.state_dir();
        let mut builder = DirBuilder::new(); builder.mode(0o700);
        let mut parent = None;
        for name in ["orchestration", "attempts"] {
            let base = parent.as_ref().unwrap_or(root);
            if create {
                match base.create_dir_with(name, &builder) {
                    Ok(()) => { base.open(".").and_then(|file| file.sync_all()).map_err(io_error)?; }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                    Err(error) => return Err(io_error(error)),
                }
            }
            let child = base.open_dir_nofollow(name).map_err(io_error)?;
            let metadata = child.dir_metadata().map_err(io_error)?;
            if metadata.uid() != nix::unistd::Uid::effective().as_raw() || metadata.mode() & 0o022 != 0 {
                return Err(conflict("attempt parent is not an owner-controlled directory"));
            }
            if name == "attempts" { private_directory(&child)?; }
            parent = Some(child);
        }
        Ok((root_path.join("orchestration/attempts"), parent.ok_or_else(|| conflict("attempt parent is unavailable"))?))
    }
}

fn private_directory(directory: &Dir) -> Result<(), InspectionError> {
    let metadata = directory.dir_metadata().map_err(io_error)?;
    if !metadata.is_dir() || metadata.uid() != nix::unistd::Uid::effective().as_raw() || metadata.mode() & 0o7777 != 0o700 {
        return Err(conflict("attempt directory must be private, owner-held and exactly mode 0700"));
    }
    Ok(())
}
fn conflict(message: &str) -> InspectionError { InspectionError::new("attempt_directory_conflict", message) }
fn unknown(message: &str) -> InspectionError { InspectionError::new("owned_attempt_directory_unknown", message) }
fn io_error(error: std::io::Error) -> InspectionError { InspectionError::new("attempt_directory_io", error.to_string()) }

#[cfg(test)]
mod tests;
