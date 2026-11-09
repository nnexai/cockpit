//! Conservative, read-only checks for machine work associated with a task.
//!
//! Task document validity and message delivery are checked by the caller under
//! the same store lock. This module only examines durable orchestration state.
use std::collections::HashSet;

use cockpit_protocol::orchestration::{
    AttachmentState, CleanupEffectState, CleanupState, CloseReason, Integration,
    ReportOutcome, ResourceRequestState, ResumePhase, RetainReason, RetirementState, RunStage,
};

use super::{
    claims::ClaimState,
    credentials::CredentialState,
    error,
    store::{DurableEffectReceipt, DurableOperationKind, DurableOperationRecord, HintState, OrchestrationState, ScopeReleaseOutcome},
};
use crate::InspectionError;

// Scope releases persist their completion in the effect receipt, not result.
// Bind that receipt to the same run, scope, claim and transition as cleanup.
fn scope_release_settled(state: &OrchestrationState, operation: &DurableOperationRecord) -> bool {
    if operation.kind != DurableOperationKind::ScopeRelease || operation.actor_run_id.is_some() {
        return false;
    }
    let Some(run_id) = operation.run_id.as_deref() else { return false };
    if operation.principal_key.strip_prefix("dispatcher-scope:") != Some(run_id) {
        return false;
    }
    let Some(run) = state.runs.iter().find(|run| run.run_id == run_id && run.root_id == operation.root_id) else { return false };
    let Some(attachment) = run.attachment.as_ref().filter(|a| a.state == AttachmentState::Revoked) else { return false };
    let Some(scope) = attachment.process_scope.as_ref() else { return false };
    let Some(checkout) = run.checkout.as_ref() else { return false };
    let [DurableEffectReceipt::ScopeRelease { unit, invocation_id, claim_id, outcome: ScopeReleaseOutcome::Released }] = operation.effect_receipts.as_slice() else { return false };
    unit == &scope.unit && invocation_id == &scope.invocation_id
        && run.checkout_claim_id.as_deref() == Some(claim_id.as_str())
        && state.checkout_claims.iter().any(|claim| &claim.claim_id == claim_id
            && claim.state == ClaimState::Released && claim.holder.is_none()
            && &claim.checkout == checkout
            && claim.transition_operation_id.as_deref() == Some(operation.operation_id.as_str()))
}

pub(super) fn check(state: &OrchestrationState, root_id: &str, task_id: &str) -> Result<(), InspectionError> {
    let runs: Vec<_> = state.runs.iter()
        .filter(|run| run.root_id == root_id && run.task_id.as_deref() == Some(task_id))
        .collect();
    let ids: HashSet<&str> = runs.iter().map(|run| run.run_id.as_str()).collect();
    let ineligible = |reason| Err(error("archive_ineligible", reason));
    if !runs.iter().any(|run| run.stage == RunStage::Closed && run.close_reason == Some(CloseReason::Accepted)) {
        return ineligible("Task requires a closed accepted run");
    }

    if runs.iter().any(|run| run.stage != RunStage::Closed
        || run.attachment.as_ref().is_some_and(|a| a.state != AttachmentState::Revoked)
        || run.resume.as_ref().is_some_and(|resume| !matches!(resume.phase, ResumePhase::Connected | ResumePhase::Reattached))
        || run.retirement.as_ref().is_some_and(|retirement| match &retirement.state {
            RetirementState::Retired { .. } => false,
            RetirementState::Retained { reason, .. } => matches!(reason,
                RetainReason::NativeProcessUnverifiable | RetainReason::ObservationUnavailable | RetainReason::ForegroundProcess),
            _ => true,
        })
        || (run.close_reason == Some(CloseReason::Accepted)
            && run.result.as_ref().is_some_and(|result| result.outcome == Some(ReportOutcome::Succeeded))
            && run.source_output.is_some() && matches!(run.integration, Integration::Pending))) {
        return ineligible("Task has an unsettled run, attachment, retirement, resume, or integration");
    }
    if state.task_ownership.iter().any(|o| o.root_id == root_id && o.task_id == task_id
        && (o.schedule.is_some() || o.owner.as_ref().is_some_and(|owner| !runs.iter().any(|run|
            run.run_id == owner.run_id && run.assignment_generation == Some(o.generation)
                && run.stage == RunStage::Closed && run.close_reason == Some(CloseReason::Accepted)))))
        || state.task_intents.iter().any(|i| i.root_id == root_id && i.task_id == task_id)
        || state.assignment_intents.iter().any(|i| i.root_id == root_id && i.task_id == task_id)
        || state.scope_intents.iter().any(|i| i.root_id == root_id && i.task_id == task_id)
        || state.continue_intents.iter().any(|i| i.root_id == root_id && i.task_id == task_id) {
        return ineligible("Task has an owner, schedule, or unsettled intent");
    }
    if state.inbox_hints.iter().any(|hint| ids.contains(hint.run_id.as_str()) && hint.state != HintState::Sent) {
        return ineligible("Task has an unsettled inbox hint");
    }
    if runs.iter().any(|run| run.checkout_claim_id.as_ref().is_some_and(|id|
        !state.checkout_claims.iter().any(|claim| &claim.claim_id == id && claim.state == ClaimState::Released)))
        || state.credentials.iter().any(|c| ids.contains(c.run_id.as_str()) && c.state != CredentialState::Revoked)
        || state.checkout_claims.iter().any(|claim| claim.state != ClaimState::Released
            && (claim.holder.as_ref().is_some_and(|holder| ids.contains(holder.run_id.as_str()))
                || runs.iter().any(|run| run.checkout_claim_id.as_deref() == Some(claim.claim_id.as_str())))) {
        return ineligible("Task has an active credential or checkout claim");
    }
    if state.resource_requests.iter().any(|request| ids.contains(request.holder.run_id.as_str())
        && request.state != ResourceRequestState::Released)
        || state.owned_resources.iter().any(|resource|
            (ids.contains(resource.run_id.as_str()) || resource.consumers.iter().any(|c| ids.contains(c.run_id.as_str())))
            && (!matches!(resource.cleanup.state, CleanupState::Removed)
                // Removed records may retain historical references. Only accept
                // exact, revoked incarnations with a released checkout claim;
                // shared or unprovable consumers remain blockers.
                || (!resource.consumers.is_empty() && (!ids.contains(resource.run_id.as_str())
                    || resource.consumers.iter().any(|consumer| !runs.iter().any(|run|
                        run.run_id == consumer.run_id
                            && run.attachment.as_ref().is_some_and(|attachment|
                                attachment.state == AttachmentState::Revoked
                                    && attachment.attachment_id == consumer.attachment_id
                                    && attachment.epoch == consumer.attachment_epoch)
                            && run.checkout.as_ref().is_some_and(|checkout|
                                run.checkout_claim_id.as_ref().is_some_and(|id|
                                    state.checkout_claims.iter().any(|claim| &claim.claim_id == id
                                        && claim.state == ClaimState::Released && claim.holder.is_none()
                                        && &claim.checkout == checkout)))))))
                || resource.cleanup.effects.iter().any(|e| matches!(e.state, CleanupEffectState::Intent | CleanupEffectState::Unknown)))) {
        return ineligible("Task has an unsettled resource request or cleanup");
    }
    // Many operations store only the root and a request digest, not their task.
    // An unsettled such operation cannot safely be attributed to another task.
    if state.operations.iter().any(|operation| operation.root_id == root_id
        && (operation.run_id.as_deref().is_none_or(|id| ids.contains(id))
            || operation.actor_run_id.as_deref().is_some_and(|id| ids.contains(id)))
        && ((operation.result.is_none() && !scope_release_settled(state, operation)) || operation.effect_receipts.iter().any(|receipt| match receipt {
            DurableEffectReceipt::ScopeRelease { outcome, .. } => *outcome != ScopeReleaseOutcome::Released,
            DurableEffectReceipt::Stop { retirement } => !matches!(retirement.state, RetirementState::Retired { .. } | RetirementState::Retained { .. }),
            _ => false,
        }))) {
        return ineligible("Task may have an unsettled durable operation");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::store::DurableOperationRecord;
    use cockpit_protocol::orchestration::IntentState;
    fn accepted_run(root: &str, task: &str) -> cockpit_protocol::orchestration::Run {
        serde_json::from_value(serde_json::json!({
            "session_id":"session", "brief":"work", "run_id":format!("{root}-{task}"),
            "kind":"worker", "label":"worker", "root_id":root, "parent_run_id":root,
            "task_id":task, "attempt":1, "assignment_generation":null,
            "task_revision_at_start":null,
            "launch_config":cockpit_protocol::projects::OrchestrationConfiguration::default(),
            "stage":"closed", "close_reason":"accepted", "dispatch":null, "target":null,
            "setup":null, "cleanup_policy":null, "last_report":null, "result":null,
            "source_output":null, "integration":{"state":"pending"}, "annotations":[],
            "project":null, "checkout":null, "attachment":null, "checkout_claim_id":null,
            "resume":null, "retirement":null, "created_at":"0", "updated_at":"0"
        })).unwrap()
    }

    #[test]
    fn task_requires_an_accepted_closed_run() {
        let mut state = OrchestrationState::default();
        assert!(check(&state, "root", "task").is_err());
        let mut run = accepted_run("root", "task");
        run.close_reason = Some(CloseReason::Cancelled);
        state.runs.push(run);
        assert!(check(&state, "root", "task").is_err());
        state.runs[0].close_reason = Some(CloseReason::Accepted);
        assert!(check(&state, "root", "task").is_ok());
    }

    #[test]
    fn settled_reattachment_does_not_hide_unsettled_work() {
        use cockpit_protocol::orchestration::ExecutionAttachment;
        let mut run = accepted_run("root", "task");
        run.attachment = Some(ExecutionAttachment {
            attachment_id: "attachment".into(), epoch: 1, state: AttachmentState::Revoked,
            launch_mode: None, location: None, conversation: None, process: None, shell: None,
            machine_id: None, process_scope: None, launch_attempt: 1, updated_at: "now".into(),
        });
        run.resume = Some(serde_json::from_value(serde_json::json!({
            "operation_id":"resume", "run_id":"root-task", "previous_attachment_epoch":1,
            "phase":"reattached", "cause":null, "decision_id":null,
            "replacement_attachment_epoch":null, "updated_at":"now"
        })).unwrap());
        let mut state = OrchestrationState::default();
        state.runs.push(run);
        assert!(check(&state, "root", "task").is_ok());
        for phase in [ResumePhase::Reconciling, ResumePhase::Launching, ResumePhase::Blocked] {
            state.runs[0].resume.as_mut().unwrap().phase = phase;
            assert!(check(&state, "root", "task").is_err());
        }
        state.runs[0].resume.as_mut().unwrap().phase = ResumePhase::Reattached;
        state.runs[0].attachment.as_mut().unwrap().state = AttachmentState::Connected;
        assert!(check(&state, "root", "task").is_err());
        state.runs[0].attachment.as_mut().unwrap().state = AttachmentState::Revoked;
        state.operations.push(DurableOperationRecord {
            operation_id: "resume".into(), actor_run_id: None, principal_key: "operator".into(),
            root_id: "root".into(), kind: DurableOperationKind::RunResume,
            request_digest: "digest".into(), run_id: Some("root-task".into()), resume_operation_id: None,
            result: None, effect_receipts: vec![], created_at: "now".into(), updated_at: "now".into(),
        });
        assert!(check(&state, "root", "task").is_err());
    }
    #[test]
    fn accepted_owner_must_match_closed_run_and_generation() {
        use cockpit_protocol::orchestration::{TaskOwner, TaskOwnership};
        let mut state = OrchestrationState::default();
        let mut run = accepted_run("root", "task");
        run.assignment_generation = Some(7);
        state.runs.push(run);
        state.task_ownership.push(TaskOwnership {
            root_id: "root".into(), task_id: "task".into(), generation: 7,
            owner: Some(TaskOwner { run_id: "root-task".into(), conversation: None, claim_operation_id: "claim".into() }),
            schedule: None, reopened_after_attempt: None,
        });
        assert!(check(&state, "root", "task").is_ok());
        state.task_ownership[0].generation = 8;
        assert!(check(&state, "root", "task").is_err());
        state.task_ownership[0].generation = 7;
        state.task_ownership[0].owner.as_mut().unwrap().run_id = "other".into();
        assert!(check(&state, "root", "task").is_err());
        state.task_ownership[0].owner.as_mut().unwrap().run_id = "root-task".into();
        state.task_ownership[0].schedule = Some(cockpit_protocol::orchestration::TaskSchedule {
            operation_id: "schedule".into(), principal_key: "operator".into(), target: None,
            brief: "work".into(), cleanup_policy: cockpit_protocol::orchestration::WorkerCleanupPolicy::OwnedWhenSafe,
            label: None, scheduled_at: "now".into(), hold: None,
        });
        assert!(check(&state, "root", "task").is_err());
        state.task_ownership[0].schedule = None;
        state.runs[0].stage = RunStage::Working;
        assert!(check(&state, "root", "task").is_err());
    }

    #[test]
    fn kept_resource_blocks_until_removed() {
        use cockpit_protocol::orchestration::{CleanupAuthority, CleanupItem, OwnedAttemptDirectoryIdentity,
            OwnedResourceIdentity, OwnedResourceKind, OwnedResourceRecord, ResourceOwnership};
        let mut state = OrchestrationState::default();
        state.runs.push(accepted_run("root", "task"));
        state.owned_resources.push(OwnedResourceRecord {
            resource_id: "resource".into(), root_id: "root".into(), run_id: "root-task".into(),
            kind: OwnedResourceKind::AttemptDirectory, ownership: ResourceOwnership::Owned,
            creation_operation_id: Some("create".into()),
            identity: OwnedResourceIdentity::AttemptDirectory { identity: OwnedAttemptDirectoryIdentity {
                creation_operation_id: "create".into(), root: "/tmp/resource".into(),
                device: "device".into(), inode: "inode".into(),
            }},
            authority: CleanupAuthority::OwnedWhenSafe, consumers: vec![], integration_result_id: None,
            cleanup: CleanupItem { operation_id: None, state: CleanupState::Kept, reason: None,
                effects: vec![], updated_at: "now".into() },
        });
        assert!(check(&state, "root", "task").is_err());
        state.owned_resources[0].cleanup.state = CleanupState::Removed;
        assert!(check(&state, "root", "task").is_ok());
    }

    #[test]
    fn removed_resource_accepts_only_proven_historical_consumers() {
        use cockpit_protocol::orchestration::{AttachmentReference, CheckoutIdentity, CleanupAuthority,
            CleanupEffectReceipt, CleanupItem, DirectoryIdentity, ExecutionAttachment,
            OwnedAttemptDirectoryIdentity, OwnedResourceIdentity, OwnedResourceKind, OwnedResourceRecord,
            ResourceOwnership};
        use super::super::claims::CheckoutClaim;
        let checkout = CheckoutIdentity { key: "checkout".into(), path: "/tmp/checkout".into(),
            directory_identity: DirectoryIdentity { device: "device".into(), inode: "inode".into() }, git: None };
        let mut run = accepted_run("root", "task");
        run.checkout = Some(checkout.clone());
        run.checkout_claim_id = Some("claim".into());
        run.attachment = Some(ExecutionAttachment {
            attachment_id: "attachment".into(), epoch: 1, state: AttachmentState::Revoked,
            launch_mode: None, location: None, conversation: None, process: None, shell: None,
            machine_id: None, process_scope: None, launch_attempt: 1, updated_at: "now".into(),
        });
        let mut state = OrchestrationState::default();
        state.runs.push(run);
        state.checkout_claims.push(CheckoutClaim { claim_id: "claim".into(), checkout, revision: 1,
            state: ClaimState::Released, holder: None, transition_operation_id: None });
        let consumer = AttachmentReference { run_id: "root-task".into(),
            attachment_id: "attachment".into(), attachment_epoch: 1 };
        state.owned_resources.push(OwnedResourceRecord {
            resource_id: "resource".into(), root_id: "root".into(), run_id: "root-task".into(),
            kind: OwnedResourceKind::AttemptDirectory, ownership: ResourceOwnership::Owned,
            creation_operation_id: Some("create".into()),
            identity: OwnedResourceIdentity::AttemptDirectory { identity: OwnedAttemptDirectoryIdentity {
                creation_operation_id: "create".into(), root: "/tmp/resource".into(),
                device: "device".into(), inode: "inode".into(),
            }},
            authority: CleanupAuthority::OwnedWhenSafe, consumers: vec![consumer.clone()],
            integration_result_id: None,
            cleanup: CleanupItem { operation_id: None, state: CleanupState::Removed, reason: None,
                effects: vec![], updated_at: "now".into() },
        });
        assert!(check(&state, "root", "task").is_ok());
        state.owned_resources[0].cleanup.state = CleanupState::Waiting;
        assert!(check(&state, "root", "task").is_err());
        state.owned_resources[0].cleanup.state = CleanupState::Removed;
        state.owned_resources[0].cleanup.effects.push(CleanupEffectReceipt {
            effect_id: "effect".into(), kind: OwnedResourceKind::AttemptDirectory,
            identity_digest: "digest".into(), state: CleanupEffectState::Unknown,
            evidence: None, updated_at: "now".into(),
        });
        assert!(check(&state, "root", "task").is_err());
        state.owned_resources[0].cleanup.effects[0].state = CleanupEffectState::Intent;
        assert!(check(&state, "root", "task").is_err());
        state.owned_resources[0].cleanup.effects[0].state = CleanupEffectState::Applied;
        state.owned_resources[0].consumers[0].attachment_epoch = 2;
        assert!(check(&state, "root", "task").is_err());
        state.owned_resources[0].consumers[0] = consumer.clone();
        state.owned_resources[0].consumers.push(AttachmentReference { run_id: "other".into(), ..consumer });
        assert!(check(&state, "root", "task").is_err());
        state.owned_resources[0].consumers.pop();
        state.owned_resources[0].run_id = "other".into();
        assert!(check(&state, "root", "task").is_err());
        state.owned_resources[0].run_id = "root-task".into();
        state.checkout_claims[0].state = ClaimState::Held;
        assert!(check(&state, "root", "task").is_err());
        state.checkout_claims[0].state = ClaimState::Released;
        state.runs[0].attachment.as_mut().unwrap().state = AttachmentState::Connected;
        assert!(check(&state, "root", "task").is_err());
    }



    #[test]
    fn task_intents_are_scoped_and_block_even_when_conflicted() {
        let mut state = OrchestrationState::default();
        state.runs.extend([accepted_run("root", "task"), accepted_run("root", "other"), accepted_run("other", "task")]);
        state.task_intents.push(cockpit_protocol::orchestration::TaskIntent {
            intent_id: "intent".into(), root_id: "root".into(), task_id: "task".into(),
            run_id: "run".into(), expected_task_revision: "revision".into(),
            state: IntentState::Conflict, origin: None, supervisor_run_id: None,
            conversation: None, result_message_id: None,
        });
        assert!(check(&state, "root", "task").is_err());
        assert!(check(&state, "root", "other").is_ok());
        assert!(check(&state, "other", "task").is_ok());
    }

    #[test]
    fn unattributed_pending_operations_block_their_root() {
        let mut state = OrchestrationState::default();
        state.runs.extend([accepted_run("root", "task"), accepted_run("other", "task")]);
        state.operations.push(DurableOperationRecord {
            operation_id: "operation".into(), actor_run_id: None, principal_key: "operator".into(),
            root_id: "root".into(), kind: DurableOperationKind::TaskScopeUpdate,
            request_digest: "digest".into(), run_id: None, resume_operation_id: None,
            result: None, effect_receipts: vec![], created_at: "now".into(), updated_at: "now".into(),
        });
        assert!(check(&state, "root", "task").is_err());
        assert!(check(&state, "other", "task").is_ok());
    }

    #[test]
    fn released_scope_receipt_without_matching_run_and_claim_is_not_settled() {
        let state = OrchestrationState::default();
        let operation = DurableOperationRecord {
            operation_id: "operation".into(), actor_run_id: None,
            principal_key: "dispatcher-scope:run".into(), root_id: "root".into(),
            kind: DurableOperationKind::ScopeRelease, request_digest: "digest".into(),
            run_id: Some("run".into()), resume_operation_id: None, result: None,
            effect_receipts: vec![DurableEffectReceipt::ScopeRelease {
                unit: "unit".into(), invocation_id: "invocation".into(), claim_id: "claim".into(),
                outcome: ScopeReleaseOutcome::Released,
            }],
            created_at: "now".into(), updated_at: "now".into(),
        };
        assert!(!scope_release_settled(&state, &operation));
    }
}
