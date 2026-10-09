use std::sync::atomic::Ordering;

use cockpit_protocol::project_teardown::{
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOutcome, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownRecovery, WorkspaceTeardownRecoveryList,
    WorkspaceTeardownRecoveryState, WorkspaceTeardownResult,
};
use cockpit_protocol::projects::{WorkspaceOperation, WorkspaceSetupMode};

use crate::InspectionError;
use crate::project_adapter::ProjectWorktreeRemoveRequest;
use crate::project_store::{TeardownReceipt, TeardownReceiptState, timestamp};
use crate::project_teardown::{
    self, WorkspaceTeardownCommand, WorkspaceTeardownEvidence, WorkspaceTeardownWorktree,
};

use super::{ProjectService, validate_session, verify_inventory};

struct FreshTeardownEvidence {
    operation: WorkspaceOperation,
    session_id: String,
    endpoint_identity: String,
    repository_key: String,
    repository_root: String,
    worktrees: Vec<WorkspaceTeardownWorktree>,
    receipt: Option<TeardownReceipt>,
}

impl FreshTeardownEvidence {
    fn evidence(&self) -> WorkspaceTeardownEvidence<'_> {
        WorkspaceTeardownEvidence {
            operation: &self.operation,
            session_id: &self.session_id,
            endpoint_identity: &self.endpoint_identity,
            repository_key: &self.repository_key,
            repository_root: &self.repository_root,
            worktrees: &self.worktrees,
            receipt: self.receipt.as_ref(),
        }
    }
}

impl ProjectService {
    /// Obtain a fresh, non-mutating teardown preview. Git status failures are
    /// represented as `unknown` evidence, which safely blocks removal.
    pub async fn teardown_preview(
        &self,
        session: &str,
        request: &WorkspaceTeardownPreviewRequest,
    ) -> Result<WorkspaceTeardownPreview, InspectionError> {
        validate_session(session)?;
        let operation =
            self.load_teardown_operation_for_workspace(session, &request.workspace_id)?;
        let evidence = self.fresh_teardown_evidence(session, operation).await?;
        project_teardown::preview(request, evidence.evidence())
    }

    /// List durable cleanup records for this session. Entries are journal
    /// pointers only; opening one must still obtain a fresh teardown preview.
    pub fn teardown_recoveries(
        &self,
        session: &str,
    ) -> Result<WorkspaceTeardownRecoveryList, InspectionError> {
        validate_session(session)?;
        let mut recoveries = Vec::new();
        for operation in self.store.list()? {
            if operation.session_id != session {
                continue;
            }
            let Some(receipt) = self.store.read_teardown_receipt(&operation.operation_id)? else {
                continue;
            };
            let Some(workspace_id) = operation.workspace_id.as_deref() else {
                continue;
            };
            if receipt.operation_id != operation.operation_id
                || receipt.workspace_id != workspace_id
                || receipt.endpoint_identity != operation.plan.endpoint_identity
                || receipt.checkout_path != operation.plan.checkout_path
                || receipt.session_id != operation.session_id
                || receipt.session_id != operation.plan.session_id
                || receipt.repository_key
                    != operation
                        .plan
                        .repository
                        .as_ref()
                        .map(|repository| repository.common_dir.as_str())
                        .unwrap_or("")
                || receipt.repository_root
                    != operation
                        .plan
                        .repository
                        .as_ref()
                        .map(|repository| repository.root.as_str())
                        .unwrap_or("")
            {
                return Err(InspectionError::new(
                    "association_conflict",
                    "teardown receipt differs from its operation journal",
                ));
            }
            let state = match receipt.state {
                TeardownReceiptState::Pending => WorkspaceTeardownRecoveryState::Pending,
                TeardownReceiptState::OutcomeUnknown => {
                    WorkspaceTeardownRecoveryState::OutcomeUnknown
                }
                TeardownReceiptState::Completed => continue,
            };
            recoveries.push(WorkspaceTeardownRecovery {
                operation_id: operation.operation_id,
                workspace_id: workspace_id.to_owned(),
                checkout_path: operation.plan.checkout_path,
                state,
            });
        }
        recoveries.sort_by(|left, right| {
            left.checkout_path
                .cmp(&right.checkout_path)
                .then(left.operation_id.cmp(&right.operation_id))
        });
        Ok(WorkspaceTeardownRecoveryList { recoveries })
    }

    /// Execute one explicitly reviewed teardown action. Worktree removal is
    /// always non-force; unrelated directories and existing notes are retained.
    pub async fn teardown_execute(
        &self,
        session: &str,
        request: &WorkspaceTeardownExecuteRequest,
    ) -> Result<WorkspaceTeardownResult, InspectionError> {
        validate_session(session)?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(InspectionError::new(
                "shutdown",
                "project service is shutting down",
            ));
        }
        let operation = self.load_teardown_operation(session, &request.operation_id)?;
        let _lease = self
            .store
            .acquire_execution_lease(&operation.operation_id)?;
        // Reload after acquiring the cross-host lease so a prior executor's
        // durable receipt is always part of the command decision.
        let operation = self.load_teardown_operation(session, &request.operation_id)?;
        let evidence = self.fresh_teardown_evidence(session, operation).await?;
        let command = project_teardown::command(request, evidence.evidence())?;
        match command {
            WorkspaceTeardownCommand::CloseSpace {
                workspace_id,
                endpoint_identity,
            } => {
                self.adapter
                    .project_close_workspace(session, &endpoint_identity, &workspace_id)
                    .await?;
                Ok(WorkspaceTeardownResult {
                    operation_id: request.operation_id.clone(),
                    workspace_id,
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: "workspace closed; the checkout was retained".to_owned(),
                })
            }
            WorkspaceTeardownCommand::RemoveOwnedWorktree {
                workspace_id,
                endpoint_identity,
                checkout_path,
                force,
                ..
            } => {
                let mut receipt = self.reviewed_teardown_receipt(&evidence)?;
                receipt.state = TeardownReceiptState::Pending;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                let remove = ProjectWorktreeRemoveRequest {
                    endpoint_identity,
                    workspace_id: workspace_id.clone(),
                    checkout_path,
                    force,
                };
                match self.adapter.project_remove_worktree(session, &remove).await {
                    Ok(()) => {}
                    Err(_) => {
                        receipt.state = TeardownReceiptState::OutcomeUnknown;
                        receipt.updated_at = timestamp();
                        // If this write fails, the prior pending receipt remains
                        // durable and still prohibits a blind redispatch.
                        self.store.write_teardown_receipt(&receipt)?;
                        return Ok(WorkspaceTeardownResult {
                            operation_id: request.operation_id.clone(),
                            workspace_id,
                            action: request.action,
                            outcome: WorkspaceTeardownOutcome::OutcomeUnknown,
                            message: "Herdr did not confirm removal; reconcile before retrying"
                                .to_owned(),
                        });
                    }
                }
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: request.operation_id.clone(),
                    workspace_id,
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: "Herdr removed the owned worktree without force".to_owned(),
                })
            }
            WorkspaceTeardownCommand::ReconcileRemoveOutcome { operation_id } => {
                self.reconcile_teardown_removal(session, &operation_id, request, &evidence)
                    .await
            }
        }
    }

    fn load_teardown_operation(
        &self,
        session: &str,
        operation_id: &str,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let operation = self.store.load(operation_id)?;
        if operation.session_id != session {
            return Err(InspectionError::new(
                "stale_identity",
                "operation belongs to another session",
            ));
        }
        Ok(operation)
    }

    fn load_teardown_operation_for_workspace(
        &self,
        session: &str,
        workspace_id: &str,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let matches = self
            .store
            .list()?
            .into_iter()
            .filter(|operation| {
                operation.session_id == session
                    && operation.workspace_id.as_deref() == Some(workspace_id)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [operation] => Ok(operation.clone()),
            [] => Err(InspectionError::new(
                "workspace_operation_not_found",
                "no Cockpit operation is associated with this workspace",
            )),
            _ => Err(InspectionError::new(
                "association_conflict",
                "multiple Cockpit operations are associated with this workspace",
            )),
        }
    }

    fn reviewed_teardown_receipt(
        &self,
        evidence: &FreshTeardownEvidence,
    ) -> Result<TeardownReceipt, InspectionError> {
        Ok(TeardownReceipt {
            operation_id: evidence.operation.operation_id.clone(),
            workspace_id: evidence.operation.workspace_id.clone().ok_or_else(|| {
                InspectionError::new("stale_identity", "operation no longer has a workspace")
            })?,
            endpoint_identity: evidence.endpoint_identity.clone(),
            session_id: evidence.session_id.clone(),
            repository_key: evidence.repository_key.clone(),
            repository_root: evidence.repository_root.clone(),
            checkout_path: evidence.operation.plan.checkout_path.clone(),
            state: TeardownReceiptState::Pending,
            updated_at: timestamp(),
        })
    }

    fn recovery_receipt(
        &self,
        evidence: &FreshTeardownEvidence,
        expected: TeardownReceiptState,
    ) -> Result<TeardownReceipt, InspectionError> {
        let receipt = evidence.receipt.clone().ok_or_else(|| {
            InspectionError::new(
                "teardown_receipt_missing",
                "teardown recovery receipt is missing",
            )
        })?;
        if receipt.state != expected
            || receipt.operation_id != evidence.operation.operation_id
            || receipt.workspace_id
                != evidence
                    .operation
                    .workspace_id
                    .as_deref()
                    .unwrap_or_default()
            || receipt.endpoint_identity != evidence.endpoint_identity
            || receipt.checkout_path != evidence.operation.plan.checkout_path
            || receipt.session_id != evidence.session_id
            || receipt.repository_key != evidence.repository_key
            || receipt.repository_root != evidence.repository_root
        {
            return Err(InspectionError::new(
                "stale_identity",
                "teardown recovery receipt no longer matches fresh provenance",
            ));
        }
        Ok(receipt)
    }

    async fn reconcile_teardown_removal(
        &self,
        _session: &str,
        operation_id: &str,
        request: &WorkspaceTeardownExecuteRequest,
        evidence: &FreshTeardownEvidence,
    ) -> Result<WorkspaceTeardownResult, InspectionError> {
        let mut receipt = self
            .recovery_receipt(evidence, TeardownReceiptState::OutcomeUnknown)
            .or_else(|error| {
                if error.code == "stale_identity" {
                    self.recovery_receipt(evidence, TeardownReceiptState::Pending)
                } else {
                    Err(error)
                }
            })?;
        match evidence.worktrees.as_slice() {
            [] => {
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: "fresh Herdr inventory confirms the worktree is gone".to_owned(),
                })
            }
            [worktree]
                if worktree.open_workspace_id.as_deref() == Some(request.workspace_id.as_str()) =>
            {
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Retained,
                    message: "fresh Herdr inventory confirms the worktree remains; request a new removal review before dispatching again".to_owned(),
                })
            }
            _ => Ok(WorkspaceTeardownResult {
                operation_id: operation_id.to_owned(),
                workspace_id: request.workspace_id.clone(),
                action: request.action,
                outcome: WorkspaceTeardownOutcome::OutcomeUnknown,
                message: "fresh Herdr inventory is ambiguous; removal remains blocked".to_owned(),
            }),
        }
    }

    async fn fresh_teardown_evidence(
        &self,
        session: &str,
        operation: WorkspaceOperation,
    ) -> Result<FreshTeardownEvidence, InspectionError> {
        if operation.plan.mode == WorkspaceSetupMode::Open || operation.plan.repository.is_none() {
            let endpoint_identity = self.adapter.project_endpoint_identity(session).await?;
            return Ok(FreshTeardownEvidence {
                session_id: session.to_owned(),
                receipt: self.store.read_teardown_receipt(&operation.operation_id)?,
                endpoint_identity,
                repository_key: String::new(),
                repository_root: String::new(),
                worktrees: vec![WorkspaceTeardownWorktree {
                    checkout_path: operation.plan.checkout_path.clone(),
                    open_workspace_id: operation.workspace_id.clone(),
                    is_linked_worktree: false,
                    dirty: None,
                }],
                operation,
            });
        }
        let repository = operation.plan.repository.as_ref().expect("checked above");
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, repository)?;
        let matching_count = inventory
            .worktrees
            .iter()
            .filter(|entry| entry.checkout_path == operation.plan.checkout_path)
            .count();
        let dirty = if matching_count == 1 {
            self.adapter
                .project_worktree_dirty(
                    &operation.plan.checkout_path,
                    self.configuration.limits.git_timeout_ms,
                    self.configuration.limits.git_output_bytes,
                )
                .await
                .ok()
        } else {
            None
        };
        let worktrees = inventory
            .worktrees
            .into_iter()
            .filter(|entry| entry.checkout_path == operation.plan.checkout_path)
            .map(|entry| WorkspaceTeardownWorktree {
                checkout_path: entry.checkout_path,
                open_workspace_id: entry.open_workspace_id,
                is_linked_worktree: entry.is_linked_worktree,
                dirty,
            })
            .collect();
        let receipt = self.store.read_teardown_receipt(&operation.operation_id)?;
        Ok(FreshTeardownEvidence {
            session_id: session.to_owned(),
            endpoint_identity: inventory.endpoint_identity,
            repository_key: inventory.repository_key,
            repository_root: inventory.repository_root,
            operation,
            worktrees,
            receipt,
        })
    }
}
