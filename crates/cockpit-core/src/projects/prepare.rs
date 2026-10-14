use std::path::Path;

use cockpit_protocol::library::{SpaceAddRequest, SpaceRepositoriesRequest, SpaceTarget};
use cockpit_protocol::orchestration::{OwnedResourceIdentity, OwnedSpaceIdentity};
use cockpit_protocol::projects::{BindMovedSpaceRequest, DirectoryCheckoutRequest, OwnedCheckoutRequest, WorkspaceCheckoutOwnership,
    WorkspaceOperation, WorkspaceOperationState, WorkspaceOperationStep, WorkspaceOwnedResource,
    WorkspaceSetupMode, WorkspaceSetupPlan};
use cockpit_protocol::v1::SessionSnapshotResponse;

use crate::{InspectionError, library::LibraryService, project_store::{OwnedCheckoutJournal, OwnedCreationReceipt},
    repositories::RepositoryCatalog};
use super::{ProjectService, error_response, validate_session, validate_text};

impl ProjectService {

    /// Borrow an existing directory for a headless actor placement; never create or own the directory.
    pub async fn prepare_directory(&self, session: &str, request: &DirectoryCheckoutRequest, library: &LibraryService)
        -> Result<WorkspaceOperation, InspectionError>
    {
        validate_session(session)?;
        uuid::Uuid::parse_str(&request.operation_id).map_err(|_| invalid("operation_id must be a UUID"))?;
        validate_text(&request.source_workspace_id, "source_workspace_id", 256)?;
        validate_text(&request.checkout_path, "checkout_path", 4096)?;
        super::plan::accessible_directory(&request.checkout_path)?;
        let canonical = std::fs::canonicalize(&request.checkout_path)
            .map_err(|e| InspectionError::new("directory_unavailable", e.to_string()))?;
        if canonical != Path::new(&request.checkout_path) {
            return Err(conflict("borrowed directory must be its exact canonical absolute path"));
        }
        let _lease = self.store.acquire_execution_lease(&request.operation_id)?;
        let prior = self.store.list()?.into_iter().find(|o| o.operation_id == request.operation_id);
        let endpoint = self.adapter.project_endpoint_identity(session).await?;
        let journal_request = OwnedCheckoutRequest {
            operation_id: request.operation_id.clone(), repository_checkout: request.checkout_path.clone(),
            checkout_path: request.checkout_path.clone(), branch: String::new(), base: String::new(),
            artifact: None, source_workspace_id: request.source_workspace_id.clone(),
        };
        if let Some(operation) = &prior
            && let Some(journal) = self.store.owned_checkout(&request.operation_id)?
        {
            if journal.attempt_directory.is_some() || !journal.borrowed_directory || journal.request != journal_request
                || operation.session_id != session || operation.plan.endpoint_identity != endpoint || operation.cancel_requested
            { return Err(conflict("borrowed directory request differs from its retained operation")); }
            return Ok(operation.clone());
        }
        let snapshot = self.adapter.session_snapshot(session).await?;
        if snapshot.session_id != session || snapshot.spaces.iter().filter(|s| s.id == request.source_workspace_id).count() != 1
            || self.adapter.project_endpoint_identity(session).await? != endpoint
        { return Err(conflict("borrowed setup has no exact source Space and endpoint")); }
        let context = library.space_listing(&SpaceTarget {
            session_id: session.into(), space_id: request.source_workspace_id.clone(),
        }).await?;
        if self.adapter.project_endpoint_identity(session).await? != endpoint {
            return Err(conflict("source context endpoint changed"));
        }
        let operation = match prior {
            Some(operation) => {
                if operation.session_id != session || operation.state != WorkspaceOperationState::Planned
                    || operation.step != WorkspaceOperationStep::Planned || operation.sequence != 0
                    || operation.plan.mode != WorkspaceSetupMode::Open
                    || operation.plan.ownership != WorkspaceCheckoutOwnership::BorrowedDirectory
                    || operation.plan.checkout_path != request.checkout_path || operation.plan.endpoint_identity != endpoint
                { return Err(conflict("existing Open plan differs from the borrowed directory request")); }
                operation
            }
            None => self.store.persist_plan(WorkspaceSetupPlan {
                operation_id: request.operation_id.clone(), generation: 1, endpoint_identity: endpoint,
                session_id: session.into(), repository: None, mode: WorkspaceSetupMode::Open,
                ownership: WorkspaceCheckoutOwnership::BorrowedDirectory, branch: None, base: None,
                checkout_path: request.checkout_path.clone(), label: canonical.file_name().and_then(|s| s.to_str()).unwrap_or("Directory").into(),
                focus: false, artifact: None, linked_artifacts: vec![], effects: vec!["borrow existing directory".into()], warnings: vec![],
            })?,
        };
        self.store.update_owned_checkout(&request.operation_id, Some(operation.generation), |o, retained| {
            *retained = Some(OwnedCheckoutJournal {
                request: journal_request, borrowed_directory: true,
                attempt_directory: None, directory_requested: false,
                resume_binding: None,
                existing_workspace_ids: snapshot.spaces.into_iter().map(|s| s.id).collect(),
                existing_terminal_ids: snapshot.panes.into_iter().map(|p| p.terminal_id).collect(),
                context_item_ids: context.items.into_iter().map(|i| i.item_id).collect(),
                context_repository_paths: context.repository_paths, git_requested: false, bind: None,
            });
            o.owned_resources.push(WorkspaceOwnedResource { kind: "directory".into(), path: request.checkout_path.clone(), created_by_operation: false });
            o.step = WorkspaceOperationStep::CheckoutPrepared; o.state = WorkspaceOperationState::Running;
            o.resume_allowed = false; o.error = None; Ok(())
        })
    }

    /// Prepare only the owned filesystem checkout. No workspace or terminal is created here.
    pub async fn prepare_owned_checkout(&self, session: &str, request: &OwnedCheckoutRequest, library: &LibraryService)
        -> Result<WorkspaceOperation, InspectionError>
    {
        validate_session(session)?;
        uuid::Uuid::parse_str(&request.operation_id).map_err(|_| invalid("operation_id must be a UUID"))?;
        validate_text(&request.source_workspace_id, "source_workspace_id", 256)?;
        let _lease = self.store.acquire_execution_lease(&request.operation_id)?;
        let prior = self.store.list()?.into_iter().find(|o| o.operation_id == request.operation_id);
        let retained = match &prior {
            Some(_) => self.store.owned_checkout(&request.operation_id)?,
            None => None,
        };
        let journal = if let Some(journal) = retained {
            if journal.attempt_directory.is_some() || journal.borrowed_directory || journal.request != *request || prior.as_ref().is_none_or(|o| o.session_id != session) {
                return Err(conflict("owned checkout request differs from its durable operation"));
            }
            journal
        } else {
            let catalog = RepositoryCatalog::new(self.configuration.clone());
            let checkout = catalog.discover_checkout(Path::new(&request.repository_checkout)).await?;
            let mut repositories = catalog.list().await?.repositories.into_iter().filter(|r|
                !r.is_linked_worktree && r.checkout_path == checkout.checkout_path && r.common_dir == checkout.common_dir);
            let repository = repositories.next().ok_or_else(|| invalid("owned checkout requires a configured primary repository"))?;
            if repositories.next().is_some() || repository.checkout_path != request.repository_checkout {
                return Err(conflict("repository checkout is not one exact configured primary"));
            }
            catalog.validate_branch(&repository, &request.branch).await?;
            let base = catalog.resolve_base(&repository, &request.base).await?;
            let checkout_path = super::plan::bounded_path(&request.checkout_path, Path::new(&self.configuration.worktree_root), "checkout_path")?;
            let endpoint = self.adapter.project_endpoint_identity(session).await?;
            let snapshot = self.adapter.session_snapshot(session).await?;
            if snapshot.session_id != session || snapshot.spaces.iter().filter(|s| s.id == request.source_workspace_id).count() != 1
                || self.adapter.project_endpoint_identity(session).await? != endpoint
            {
                return Err(conflict("source Space or endpoint changed while preparing checkout"));
            }
            let source_context = library.space_listing(&SpaceTarget {
                session_id: session.into(), space_id: request.source_workspace_id.clone(),
            }).await?;
            if self.adapter.project_endpoint_identity(session).await? != endpoint {
                return Err(conflict("source context endpoint changed"));
            }
            let operation = match prior {
                Some(operation) => {
                    let plan = &operation.plan;
                    if operation.session_id != session || operation.state != WorkspaceOperationState::Planned
                        || operation.step != WorkspaceOperationStep::Planned || operation.sequence != 0
                        || plan.mode != WorkspaceSetupMode::Create || plan.ownership != WorkspaceCheckoutOwnership::OwnedWorktree
                        || plan.endpoint_identity != endpoint || plan.checkout_path != checkout_path
                        || plan.branch.as_deref() != Some(request.branch.as_str()) || plan.base.as_deref() != Some(base.as_str())
                        || plan.artifact != request.artifact || plan.repository.as_ref().is_none_or(|r|
                            r.checkout_path != repository.checkout_path || r.common_dir != repository.common_dir || r.root != repository.root)
                    {
                        return Err(conflict("existing plan differs from the owned checkout request"));
                    }
                    operation
                }
                None => self.store.persist_plan(WorkspaceSetupPlan {
                    operation_id: request.operation_id.clone(), generation: 1, endpoint_identity: endpoint,
                    session_id: session.into(), repository: Some(repository), mode: WorkspaceSetupMode::Create,
                    ownership: WorkspaceCheckoutOwnership::OwnedWorktree, branch: Some(request.branch.clone()),
                    base: Some(base), checkout_path, label: request.branch.clone(), focus: false,
                    artifact: request.artifact.clone(), linked_artifacts: vec![],
                    effects: vec!["create owned filesystem worktree and branch".into()], warnings: vec![],
                })?,
            };
            let journal = OwnedCheckoutJournal {
                request: request.clone(), existing_workspace_ids: snapshot.spaces.into_iter().map(|s| s.id).collect(),
                borrowed_directory: false,
                attempt_directory: None, directory_requested: false,
                resume_binding: None,
                existing_terminal_ids: snapshot.panes.into_iter().map(|p| p.terminal_id).collect(),
                context_item_ids: source_context.items.into_iter().map(|i| i.item_id).collect(),
                context_repository_paths: source_context.repository_paths, git_requested: false, bind: None,
            };
            self.store.update_owned_checkout(&request.operation_id, Some(operation.generation), |operation, retained| {
                *retained = Some(journal.clone());
                operation.step = WorkspaceOperationStep::Validated;
                operation.state = WorkspaceOperationState::Running;
                operation.resume_allowed = false;
                Ok(())
            })?;
            journal
        };
        let operation = self.store.load(&request.operation_id)?;
        if operation.cancel_requested { return Err(invalid("owned checkout operation was cancelled")); }
        if self.adapter.project_endpoint_identity(session).await? != operation.plan.endpoint_identity {
            return Err(conflict("owned checkout endpoint changed"));
        }
        if operation.state == WorkspaceOperationState::Completed { return Ok(operation); }
        if let Some(receipt) = self.store.read_owned_creation(&request.operation_id)? {
            crate::project_teardown::validate_creation_receipt(&receipt, &operation)?;
            // Re-inspection confirms retained physical proof; it never grants ownership to a recovered path.
            let identity = receipt.resources.iter().find_map(|r| match r {
                OwnedResourceIdentity::Worktree { identity } => Some(identity), _ => None,
            }).ok_or_else(|| unknown("owned Git creation proof is unavailable"))?;
            let branch = operation.plan.branch.as_ref().map(|b| format!("refs/heads/{b}"));
            let current = crate::project_adapter::capture_owned_git_identity(&request.operation_id,
                &identity.repository_root, &identity.checkout_path, branch.as_deref(),
                self.configuration.limits.git_timeout_ms as u64, self.configuration.limits.git_output_bytes as usize).await?;
            if current != *identity { return Err(conflict("prepared checkout physical identity changed")); }
            if operation.workspace_id.is_some() || operation.step == WorkspaceOperationStep::CheckoutPrepared {
                return Ok(operation);
            }
            return self.store.update(&request.operation_id, Some(operation.generation), |o| {
                o.step = WorkspaceOperationStep::CheckoutPrepared; o.state = WorkspaceOperationState::Running;
                o.error = None; o.resume_allowed = false; Ok(())
            });
        }
        if journal.git_requested {
            return Err(unknown("interrupted Git creation has no captured physical receipt; existing resources are protected, never replayed"));
        }
        let repository = operation.plan.repository.as_ref().ok_or_else(|| invalid("prepared plan has no repository"))?;
        let fresh = RepositoryCatalog::new(self.configuration.clone()).resolve(&repository.repository_id).await?;
        if fresh.checkout_path != repository.checkout_path || fresh.root != repository.root || fresh.common_dir != repository.common_dir {
            return Err(conflict("repository physical routing changed"));
        }
        match std::fs::symlink_metadata(&operation.plan.checkout_path) {
            Ok(_) => return Err(conflict("owned checkout destination already exists")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(InspectionError::new("path_unavailable", e.to_string())),
        }
        if RepositoryCatalog::new(self.configuration.clone()).local_branch_commit(repository, &request.branch).await?.is_some() {
            return Err(conflict("owned branch already exists"));
        }
        super::plan::bounded_path(&operation.plan.checkout_path, Path::new(&self.configuration.worktree_root), "checkout_path")?;
        self.store.update_owned_checkout(&request.operation_id, Some(operation.generation), |o, journal| {
            journal.as_mut().ok_or_else(|| conflict("owned checkout journal is missing"))?.git_requested = true;
            o.error = Some(error_response("owned_checkout_git_pending", "awaiting local Git creation outcome"));
            o.state = WorkspaceOperationState::OutcomeUnknown; o.resume_allowed = false; Ok(())
        })?;
        let result = crate::space_git::git_output(Path::new(&repository.checkout_path), &[
            "worktree", "add", "-b", &request.branch, "--", &operation.plan.checkout_path,
            operation.plan.base.as_deref().ok_or_else(|| invalid("checkout base is missing"))?,
        ]).await;
        let output = match result {
            Ok(output) if output.status.success() => output,
            Ok(output) => return Err(unknown(&crate::space_git::safe_detail(&String::from_utf8_lossy(&output.stderr)))),
            Err(error) => return Err(error),
        };
        drop(output);
        self.store.update(&request.operation_id, None, |o| {
            o.owned_resources.push(WorkspaceOwnedResource { kind: "worktree".into(),
                path: o.plan.checkout_path.clone(), created_by_operation: true });
            Ok(())
        })?;
        let branch_ref = format!("refs/heads/{}", request.branch);
        let identity = crate::project_adapter::capture_owned_git_identity(&request.operation_id,
            &repository.root, &operation.plan.checkout_path, Some(&branch_ref),
            self.configuration.limits.git_timeout_ms as u64, self.configuration.limits.git_output_bytes as usize).await?;
        self.store.write_owned_creation(&OwnedCreationReceipt {
            operation_id: request.operation_id.clone(), resources: vec![
                OwnedResourceIdentity::Worktree { identity: identity.clone() }, OwnedResourceIdentity::Branch { identity },
            ], git_capture_error: None, moved_space: None,
        })?;
        self.store.update(&request.operation_id, None, |o| {
            o.step = WorkspaceOperationStep::CheckoutPrepared; o.state = WorkspaceOperationState::Running;
            o.error = None; o.resume_allowed = false; Ok(())
        })
    }

    /// Bind the exact single terminal moved into a freshly created Space, then select existing Library paths.
    pub async fn bind_moved_space(&self, session: &str, request: &BindMovedSpaceRequest, library: &LibraryService)
        -> Result<WorkspaceOperation, InspectionError>
    {
        validate_session(session)?;
        for (name, value) in [("workspace_id", &request.workspace_id), ("tab_id", &request.tab_id),
            ("pane_id", &request.pane_id), ("terminal_id", &request.terminal_id), ("endpoint_identity", &request.endpoint_identity)]
        { validate_text(value, name, 256)?; }
        uuid::Uuid::parse_str(&request.move_operation_id).map_err(|_| invalid("move_operation_id must be a UUID"))?;
        let _lease = self.store.acquire_execution_lease(&request.operation_id)?;
        let operation = self.store.load(&request.operation_id)?;
        let journal = self.store.owned_checkout(&request.operation_id)?.ok_or_else(|| invalid("operation is not an owned checkout preparation"))?;
        if operation.session_id != session || operation.plan.endpoint_identity != request.endpoint_identity || operation.cancel_requested {
            return Err(conflict("bind session, endpoint or operation changed"));
        }
        if let Some(binding) = &journal.bind {
            if !same_binding(binding, request) { return Err(conflict("bind request differs from retained move receipt")); }
        } else if operation.generation != request.expected_generation || operation.step != WorkspaceOperationStep::CheckoutPrepared {
            return Err(InspectionError::new("stale_generation", "bind requires the exact prepared operation generation"));
        }
        self.verify_moved_checkout(session, &operation, &journal, request).await?;
        let creation = self.store.read_owned_creation(&request.operation_id)?;
        if !journal.borrowed_directory && creation.is_none() {
            return Err(unknown("prepared checkout has no captured creation proof"));
        }
        if journal.bind.is_none() && journal.attempt_directory.is_none() && let Some(receipt) = &creation {
            crate::project_teardown::validate_creation_receipt(receipt, &operation)?;
            let identity = receipt.resources.iter().find_map(|r| match r {
                OwnedResourceIdentity::Worktree { identity } => Some(identity), _ => None,
            }).ok_or_else(|| unknown("prepared worktree physical proof is missing"))?;
            if !crate::project_adapter::inspect_owned_git(identity, &identity.creation_tip, true,
                self.configuration.limits.git_timeout_ms as u64, self.configuration.limits.git_output_bytes as usize).await?
            { return Err(conflict("prepared worktree is no longer present")); }
        }
        if journal.attempt_directory.is_some() {
            let identity = creation.as_ref().and_then(|r| r.resources.iter().find_map(|r| match r {
                OwnedResourceIdentity::AttemptDirectory { identity } => Some(identity), _ => None,
            })).ok_or_else(|| unknown("attempt directory has no captured physical creation proof"))?;
            self.validate_attempt_directory_identity(identity)?;
        }
        if journal.bind.is_none() {
            self.store.update_owned_checkout(&request.operation_id, Some(request.expected_generation), |o, journal| {
                journal.as_mut().ok_or_else(|| conflict("owned checkout journal disappeared"))?.bind = Some(request.clone());
                o.workspace_id = Some(request.workspace_id.clone()); o.tab_id = Some(request.tab_id.clone());
                o.pane_id = Some(request.pane_id.clone()); o.step = WorkspaceOperationStep::SpaceBound;
                o.state = WorkspaceOperationState::Running; o.error = None; Ok(())
            })?;
        }
        let mut receipt = creation.unwrap_or_else(|| OwnedCreationReceipt {
            operation_id: request.operation_id.clone(), resources: vec![], git_capture_error: None, moved_space: None,
        });
        if receipt.moved_space.is_none() {
            receipt.resources.push(OwnedResourceIdentity::Space { identity: OwnedSpaceIdentity {
                creation_operation_id: request.move_operation_id.clone(), endpoint_identity: request.endpoint_identity.clone(),
                session_id: session.into(), workspace_id: request.workspace_id.clone(), checkout_path: operation.plan.checkout_path.clone(),
            }});
            receipt.moved_space = journal.bind.clone().or_else(|| Some(request.clone()));
            self.store.write_owned_creation(&receipt)?;
        }
        if operation.state == WorkspaceOperationState::Completed { return self.store.load(&request.operation_id); }
        let result = self.select_checkout_context(session, &operation, &journal, request, library).await;
        if let Err(error) = result {
            self.store.update(&request.operation_id, None, |o| {
                o.state = WorkspaceOperationState::Partial; o.step = WorkspaceOperationStep::SpaceBound;
                o.resume_allowed = false; o.error = Some(error_response(&error.code, &error.message)); Ok(())
            })?;
            return Err(error);
        }
        // No launch or context operation creates a tab: the receipt still has exactly the worker pane.
        self.verify_moved_checkout(session, &operation, &journal, request).await?;
        self.store.update(&request.operation_id, None, |o| {
            o.state = WorkspaceOperationState::Completed; o.step = WorkspaceOperationStep::Completed;
            o.error = None; o.resume_allowed = false; Ok(())
        })
    }

    async fn verify_moved_checkout(&self, session: &str, operation: &WorkspaceOperation,
        journal: &OwnedCheckoutJournal, request: &BindMovedSpaceRequest) -> Result<(), InspectionError>
    {
        if let Some(resume) = &journal.resume_binding {
            let observed = self.adapter.project_snapshot_at_endpoint(session, &request.endpoint_identity).await?;
            Self::validate_source_incarnation(&resume.source_location, &observed)?;
            validate_moved_snapshot(&observed.snapshot, operation, journal, request)?;
            if !observed.snapshot.tabs.iter().any(|tab| tab.id == request.tab_id && tab.label == resume.source_location.launch_tag) {
                return Err(conflict("resumed actor tab does not match its pre-move launch tag"));
            }
            self.validate_resume_checkout(resume.previous_operation_id.as_deref(),
                &resume.expected_checkout, operation.plan.base.as_deref()).await?;
            return Ok(());
        }
        if self.adapter.project_endpoint_identity(session).await? != request.endpoint_identity {
            return Err(conflict("move endpoint changed"));
        }
        let snapshot = self.adapter.session_snapshot(session).await?;
        validate_moved_snapshot(&snapshot, operation, journal, request)?;
        if self.adapter.project_endpoint_identity(session).await? != request.endpoint_identity {
            return Err(conflict("move endpoint changed during fresh receipt inspection"));
        }
        Ok(())
    }

    async fn select_checkout_context(&self, session: &str, operation: &WorkspaceOperation,
        journal: &OwnedCheckoutJournal, request: &BindMovedSpaceRequest, library: &LibraryService) -> Result<(), InspectionError>
    {
        let target = SpaceTarget { session_id: session.into(), space_id: request.workspace_id.clone() };
        if !journal.context_item_ids.is_empty() {
            let selection = library.start_space_add(SpaceAddRequest { target: target.clone(), item_ids: journal.context_item_ids.clone() }).await?;
            loop {
                let response = library.operation(&selection.operation_id).await?;
                if response.finished {
                    if response.phases.iter().any(|p| p.state != cockpit_protocol::library::LibraryPhaseState::Done) {
                        return Err(InspectionError::new("source_sync_conflict", "borrowed source Space Library selection did not complete"));
                    }
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
        if !journal.context_repository_paths.is_empty() {
            library.space_repositories(SpaceRepositoriesRequest { target,
                repository_paths: journal.context_repository_paths.clone() }).await?;
        }
        self.materialize_context(session, &operation.plan, &request.workspace_id, library, vec![]).await
    }
}

pub(super) fn validate_moved_snapshot(snapshot: &SessionSnapshotResponse, operation: &WorkspaceOperation,
    journal: &OwnedCheckoutJournal, request: &BindMovedSpaceRequest) -> Result<(), InspectionError>
{
    validate_moved_geometry(snapshot, &operation.session_id, &operation.plan.checkout_path, journal, request)
}

pub(super) fn validate_moved_geometry(snapshot: &SessionSnapshotResponse, session: &str, checkout_path: &str,
    journal: &OwnedCheckoutJournal, request: &BindMovedSpaceRequest) -> Result<(), InspectionError>
{
    if snapshot.session_id != session || request.workspace_id == journal.request.source_workspace_id
        || journal.existing_workspace_ids.contains(&request.workspace_id)
        || journal.existing_terminal_ids.contains(&request.terminal_id)
        || snapshot.spaces.iter().filter(|s| s.id == request.workspace_id).count() != 1
    { return Err(conflict("move did not prove a new Space distinct from the source and all prior Spaces")); }
    let mut panes = snapshot.panes.iter().filter(|p| p.space_id == request.workspace_id);
    let pane = match (panes.next(), panes.next()) { (Some(pane), None) => pane,
        _ => return Err(conflict("moved Space must contain exactly the worker pane")), };
    let mut tabs = snapshot.tabs.iter().filter(|t| t.space_id == request.workspace_id);
    let tab = match (tabs.next(), tabs.next()) { (Some(tab), None) => tab,
        _ => return Err(conflict("moved Space must contain exactly the worker tab")), };
    if pane.id != request.pane_id || pane.terminal_id != request.terminal_id || pane.tab_id != request.tab_id
        || tab.id != request.tab_id || snapshot.panes.iter().filter(|p| p.terminal_id == request.terminal_id).count() != 1
        || pane.cwd.as_deref().and_then(|p| std::fs::canonicalize(p).ok()).as_deref() != Some(Path::new(checkout_path))
    { return Err(conflict("fresh move receipt has a foreign terminal, tab, pane or checkout")); }
    Ok(())
}

fn same_binding(a: &BindMovedSpaceRequest, b: &BindMovedSpaceRequest) -> bool {
    a.operation_id == b.operation_id && a.endpoint_identity == b.endpoint_identity && a.workspace_id == b.workspace_id
        && a.tab_id == b.tab_id && a.pane_id == b.pane_id && a.terminal_id == b.terminal_id && a.move_operation_id == b.move_operation_id
}
fn invalid(message: &str) -> InspectionError { InspectionError::new("invalid_project_request", message) }
fn conflict(message: &str) -> InspectionError { InspectionError::new("association_conflict", message) }
fn unknown(message: &str) -> InspectionError { InspectionError::new("owned_checkout_outcome_unknown", message) }

#[cfg(test)]
mod tests;
