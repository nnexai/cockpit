use std::path::Path;

use cap_std::fs::MetadataExt;
use cockpit_protocol::library::SpaceTarget;
use cockpit_protocol::orchestration::{CheckoutIdentity, OwnedGitIdentity, OwnedResourceIdentity, RunLocation};
use cockpit_protocol::projects::{BindMovedSpaceRequest, OwnedCheckoutRequest, WorkspaceCheckoutOwnership,
    WorkspaceOperation, WorkspaceSetupMode, WorkspaceSetupPlan};

use crate::{InspectionError, library::LibraryService,
    project_store::{OwnedCheckoutJournal, ResumeSpaceBinding, open_dir_nofollow_absolute}};
use super::{ProjectService, validate_session, validate_text};

impl ProjectService {
    /// Bind only a new moved Space for a reserved resumed worker incarnation.
    /// The caller fences the worker epoch/claim; the original filesystem creator never changes.
    pub async fn bind_resumed_space(
        &self, session: &str, previous_operation_id: Option<&str>, expected_checkout: &CheckoutIdentity,
        source_location: &RunLocation, request: &BindMovedSpaceRequest, library: &LibraryService,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_session(session)?;
        for (name, value) in [("workspace_id", &request.workspace_id), ("tab_id", &request.tab_id),
            ("pane_id", &request.pane_id), ("terminal_id", &request.terminal_id), ("endpoint_identity", &request.endpoint_identity)]
        { validate_text(value, name, 256)?; }
        if request.operation_id != request.move_operation_id || request.expected_generation != 1
            || uuid::Uuid::parse_str(&request.operation_id).is_err()
            || previous_operation_id == Some(request.operation_id.as_str())
            || source_location.session_id != session || source_location.endpoint_identity != request.endpoint_identity
            || source_location.workspace_id == request.workspace_id || source_location.workspace_id.is_empty()
            || source_location.tab_id.is_empty() || source_location.pane_id.is_empty() || source_location.launch_tag.is_empty()
            || source_location.terminal_id.as_deref() != Some(request.terminal_id.as_str())
        {
            return Err(conflict("resumed binding has no exact new move/source/terminal identity"));
        }
        let _creator_lease = match previous_operation_id {
            Some(id) => Some(self.store.acquire_execution_lease(id)?), None => None,
        };
        let binding_lease = self.store.acquire_execution_lease(&request.operation_id)?;
        let previous = match previous_operation_id {
            Some(id) => {
                let operation = self.store.load(id)?;
                if operation.session_id != session || operation.plan.checkout_path != expected_checkout.path
                    || self.store.owned_checkout(id)?.is_none_or(|journal| journal.resume_binding.is_some())
                { return Err(conflict("resumed binding predecessor is not the original checkout setup")); }
                Some(operation)
            }
            None => None,
        };
        let prior = self.store.list()?.into_iter().find(|operation| operation.operation_id == request.operation_id);
        let lineage = ResumeSpaceBinding { previous_operation_id: previous_operation_id.map(str::to_owned),
            expected_checkout: expected_checkout.clone(), source_location: source_location.clone() };
        if let Some(operation) = prior {
            let journal = self.store.owned_checkout(&request.operation_id)?.ok_or_else(|| conflict("resumed binding journal is missing"))?;
            let retained = journal.resume_binding.as_ref().ok_or_else(|| conflict("move ID belongs to another operation"))?;
            if retained.previous_operation_id != lineage.previous_operation_id || retained.expected_checkout != *expected_checkout
                || !same_source(&retained.source_location, source_location) || journal.bind.as_ref().is_none_or(|binding| binding != request)
                || operation.session_id != session
            { return Err(conflict("resumed binding differs from its immutable lineage")); }
            self.validate_resume_checkout(previous_operation_id, expected_checkout, operation.plan.base.as_deref()).await?;
        } else {
            let head = self.validate_resume_checkout(previous_operation_id, expected_checkout, None).await?;
            let observed = self.adapter.project_snapshot_at_endpoint(session, &request.endpoint_identity).await?;
            Self::validate_source_incarnation(source_location, &observed)?;
            let previous_journal = match previous_operation_id {
                Some(id) => self.store.owned_checkout(id)?, None => None,
            };
            let (item_ids, repository_paths) = match previous_journal.as_ref() {
                Some(journal) => (journal.context_item_ids.clone(), journal.context_repository_paths.clone()),
                None => {
                    let context = library.space_listing(&SpaceTarget { session_id: session.into(),
                        space_id: source_location.workspace_id.clone() }).await?;
                    (context.items.into_iter().map(|item| item.item_id).collect(), context.repository_paths)
                }
            };
            let mut existing_workspace_ids = previous_journal.as_ref().map(|j| j.existing_workspace_ids.clone()).unwrap_or_default();
            existing_workspace_ids.push(source_location.workspace_id.clone());
            if let Some(workspace) = previous.as_ref().and_then(|operation| operation.workspace_id.as_ref()) {
                existing_workspace_ids.push(workspace.clone());
            }
            existing_workspace_ids.sort(); existing_workspace_ids.dedup();
            let branch = previous.as_ref().and_then(|operation| operation.plan.branch.clone());
            let artifact = previous.as_ref().and_then(|operation| operation.plan.artifact.clone());
            let plan = WorkspaceSetupPlan {
                operation_id: request.operation_id.clone(), generation: 1, endpoint_identity: request.endpoint_identity.clone(),
                session_id: session.into(), repository: previous.as_ref().and_then(|operation| operation.plan.repository.clone()),
                mode: WorkspaceSetupMode::Open, ownership: WorkspaceCheckoutOwnership::BorrowedDirectory,
                branch: branch.clone(), base: head.clone(), checkout_path: expected_checkout.path.clone(),
                label: previous.as_ref().map(|operation| operation.plan.label.clone()).unwrap_or_else(|| source_location.launch_tag.clone()),
                focus: false, artifact: artifact.clone(),
                linked_artifacts: previous.as_ref().map(|operation| operation.plan.linked_artifacts.clone()).unwrap_or_default(),
                effects: vec!["bind resumed worker's newly moved Space; retain existing checkout ownership".into()], warnings: vec![],
            };
            let journal = OwnedCheckoutJournal {
                request: OwnedCheckoutRequest { operation_id: request.operation_id.clone(),
                    repository_checkout: previous.as_ref().and_then(|operation| operation.plan.repository.as_ref()).map(|repo| repo.checkout_path.clone())
                        .unwrap_or_else(|| expected_checkout.path.clone()), checkout_path: expected_checkout.path.clone(),
                    branch: branch.unwrap_or_default(), base: head.unwrap_or_default(), artifact,
                    source_workspace_id: source_location.workspace_id.clone() },
                borrowed_directory: true, attempt_directory: None, directory_requested: false,
                existing_workspace_ids, existing_terminal_ids: vec![], context_item_ids: item_ids,
                context_repository_paths: repository_paths, git_requested: false, bind: Some(request.clone()), resume_binding: Some(lineage),
            };
            // Validate the destination before the new immutable binding operation can own anything.
            super::prepare::validate_moved_geometry(&observed.snapshot, session, &plan.checkout_path, &journal, request)?;
            if !observed.snapshot.tabs.iter().any(|tab| tab.id == request.tab_id && tab.label == source_location.launch_tag) {
                return Err(conflict("new resumed tab has a foreign launch tag"));
            }
            self.store.persist_owned_checkout_plan(plan, journal)?;
        }
        // The common binder owns this operation's execution lease while selecting context/completing it.
        drop(binding_lease);
        self.bind_moved_space(session, request, library).await
    }

    pub(super) async fn validate_resume_checkout(
        &self, previous_operation_id: Option<&str>, expected: &CheckoutIdentity, pinned_head: Option<&str>,
    ) -> Result<Option<String>, InspectionError> {
        let directory = open_dir_nofollow_absolute(Path::new(&expected.path)).map_err(|error| conflict(&error.to_string()))?;
        let metadata = directory.dir_metadata().map_err(|error| conflict(&error.to_string()))?;
        if std::fs::canonicalize(&expected.path).ok().as_deref() != Some(Path::new(&expected.path))
            || metadata.dev().to_string() != expected.directory_identity.device || metadata.ino().to_string() != expected.directory_identity.inode
        { return Err(conflict("resumed checkout physical identity changed")); }
        if expected.git.is_some() {
            let (_, actual) = crate::orchestration::project_identity::resolve_directory(Path::new(&expected.path), None, None).await?;
            if actual != *expected { return Err(conflict("resumed checkout Git key/common/admin binding changed")); }
        } else if crate::orchestration::project_identity::checkout_key(&expected.path, "") != expected.key {
            return Err(conflict("resumed directory checkout key changed"));
        }
        if let Some(id) = previous_operation_id {
            let operation = self.store.load(id)?;
            let receipt = self.store.read_owned_creation(id)?.ok_or_else(|| conflict("original checkout creation receipt is unavailable"))?;
            crate::project_teardown::validate_creation_receipt(&receipt, &operation)?;
            if operation.plan.checkout_path != expected.path { return Err(conflict("original checkout path differs")); }
            match operation.plan.ownership {
                WorkspaceCheckoutOwnership::OwnedWorktree => {
                    let identity = receipt.resources.iter().find_map(|resource| match resource {
                        OwnedResourceIdentity::Worktree { identity } => Some(identity), _ => None,
                    }).ok_or_else(|| conflict("original worktree creation proof is unavailable"))?;
                    let current = crate::project_adapter::capture_owned_git_identity(id, &identity.repository_root, &identity.checkout_path,
                        identity.branch_ref.as_deref(), self.configuration.limits.git_timeout_ms as u64,
                        self.configuration.limits.git_output_bytes as usize).await.map_err(|error| conflict(&error.message))?;
                    if !same_git_creation(identity, &current) { return Err(conflict("original worktree/common-directory/branch physical proof changed")); }
                }
                WorkspaceCheckoutOwnership::OwnedAttemptDirectory => {
                    let identity = receipt.resources.iter().find_map(|resource| match resource {
                        OwnedResourceIdentity::AttemptDirectory { identity } => Some(identity), _ => None,
                    }).ok_or_else(|| conflict("original attempt directory creation proof is unavailable"))?;
                    self.validate_attempt_directory_identity(identity)?;
                }
                WorkspaceCheckoutOwnership::BorrowedDirectory => {}
            }
        }
        let head = if expected.git.is_some() {
            Some(crate::space_git::git_line(Path::new(&expected.path), &["rev-parse", "--verify", "HEAD"]).await?)
        } else { None };
        if pinned_head.is_some() && pinned_head != head.as_deref() {
            return Err(conflict("resumed checkout HEAD changed after binding intent"));
        }
        Ok(head)
    }
}

fn same_git_creation(original: &OwnedGitIdentity, current: &OwnedGitIdentity) -> bool {
    original.creation_operation_id == current.creation_operation_id && original.machine_id == current.machine_id
        && original.repository_key == current.repository_key && original.repository_root == current.repository_root
        && original.repository_device == current.repository_device && original.repository_inode == current.repository_inode
        && original.common_dir == current.common_dir && original.common_device == current.common_device && original.common_inode == current.common_inode
        && original.checkout_key == current.checkout_key && original.checkout_path == current.checkout_path
        && original.checkout_device == current.checkout_device && original.checkout_inode == current.checkout_inode
        && original.branch_ref == current.branch_ref
}
fn same_source(left: &RunLocation, right: &RunLocation) -> bool {
    left.boot_id == right.boot_id && left.peer_process == right.peer_process && left.terminal_id == right.terminal_id
        && left.native_session_id == right.native_session_id && left.endpoint_identity == right.endpoint_identity
        && left.session_id == right.session_id && left.workspace_id == right.workspace_id && left.tab_id == right.tab_id
        && left.pane_id == right.pane_id && left.launch_tag == right.launch_tag
}
fn conflict(message: &str) -> InspectionError { InspectionError::new("association_conflict", message) }
