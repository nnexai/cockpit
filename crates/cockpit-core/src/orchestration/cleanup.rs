use sha2::{Digest, Sha256};
use super::*;
use super::mutate::resume;
use super::store::DurableOperationKind;
use crate::projects::ProjectService;
mod scopes;
mod attempt_directory;
pub(crate) mod merged;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupEvaluation {
    Ready,
    Absent,
    Waiting(CleanupReason),
    Kept(CleanupReason),
    Unknown(CleanupReason),
}

#[derive(Debug, Clone)]
pub struct CleanupEffectOutcome {
    pub state: CleanupEffectState,
    pub evidence: Option<String>,
    pub reason: Option<CleanupReason>,
    pub effects: Vec<CleanupEffectReceipt>,
}

/// Opaque service-issued authority, not a wire receipt or an empty-list promise.
/// It borrows the exact evaluated resource/run and revalidates the private
/// release CAS plus physical scope drain each time an adapter consumes it.
pub struct OwnedConsumerProof<'a> {
    service: &'a OrchestrationService,
    resource: &'a OwnedResourceRecord,
    run: &'a Run,
    claim_revision: u64,
    merged: Option<&'a merged::MergedProof>,
}

impl OwnedConsumerProof<'_> {
    pub(crate) async fn validate(&self, resource: &OwnedResourceRecord, run: &Run) -> Result<(), InspectionError> {
        let refusal = || error("consumer_proof_missing", "Exact service-issued released checkout authority is absent or changed");
        if resource.resource_id != self.resource.resource_id || resource.identity != self.resource.identity
            || resource.creation_operation_id != self.resource.creation_operation_id || resource.run_id != self.run.run_id
            || resource.kind != self.resource.kind || resource.ownership != self.resource.ownership
            || resource.authority != self.resource.authority
            || resource.root_id != self.run.root_id || run.run_id != self.run.run_id
            || !consumer_run_matches(run, self.run) || run.checkout != self.run.checkout
            || run.retirement != self.run.retirement || !resource.consumers.is_empty() { return Err(refusal()); }
        self.service.validate_consumer_release(self.resource, self.run, self.claim_revision, self.merged)?;
        let scope = self.run.attachment.as_ref().and_then(|attachment| attachment.process_scope.as_ref()).ok_or_else(refusal)?;
        let observation = crate::process_scope::observe(scope).await?;
        if observation.scope != *scope || !matches!(observation.state, crate::process_scope::ScopeState::Drained) {
            return Err(error("consumer_scope_populated", "Fresh exact owned scope has not proved every contained descendant drained"));
        }
        self.service.validate_consumer_release(self.resource, self.run, self.claim_revision, self.merged)
    }
}

fn consumer_run_matches(current: &Run, expected: &Run) -> bool {
    current.run_id == expected.run_id && current.root_id == expected.root_id && current.kind == expected.kind
        && current.stage == expected.stage && current.close_reason == expected.close_reason
        && current.attempt == expected.attempt && current.assignment_generation == expected.assignment_generation
        && current.cleanup_policy == expected.cleanup_policy && current.checkout == expected.checkout
        && current.checkout_claim_id == expected.checkout_claim_id && current.integration == expected.integration
        && current.source_output == expected.source_output && current.retirement == expected.retirement
        && current.setup.as_ref().map(|setup| (&setup.operation_id, &setup.checkout_path))
            == expected.setup.as_ref().map(|setup| (&setup.operation_id, &setup.checkout_path))
        && current.attachment.as_ref().map(|attachment| (&attachment.attachment_id, attachment.epoch, attachment.state,
            attachment.launch_mode, &attachment.process, &attachment.shell, &attachment.machine_id, &attachment.process_scope, attachment.launch_attempt))
            == expected.attachment.as_ref().map(|attachment| (&attachment.attachment_id, attachment.epoch, attachment.state,
                attachment.launch_mode, &attachment.process, &attachment.shell, &attachment.machine_id, &attachment.process_scope, attachment.launch_attempt))
}

fn released_consumer_revision(state: &OrchestrationState, resource: &OwnedResourceRecord, expected: &Run) -> Option<u64> {
    use super::store::{DurableEffectReceipt, ScopeReleaseOutcome};
    let run = state.runs.iter().find(|run| run.run_id == expected.run_id)?;
    let registered = state.owned_resources.iter().find(|registered| registered.resource_id == resource.resource_id)?;
    let (OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity }) = &resource.identity else { return None };
    if !consumer_run_matches(run, expected) || run.kind != RunKind::Worker || run.stage != RunStage::Closed
        || run.close_reason != Some(CloseReason::Accepted) || run.cleanup_policy != Some(WorkerCleanupPolicy::OwnedWhenSafe)
        || registered.identity != resource.identity || registered.creation_operation_id != resource.creation_operation_id
        || registered.run_id != run.run_id || registered.root_id != run.root_id
        || registered.kind != resource.kind || resource.kind != kind(&resource.identity)
        || registered.ownership != ResourceOwnership::Owned || registered.authority != CleanupAuthority::OwnedWhenSafe
        || resource.ownership != ResourceOwnership::Owned || resource.authority != CleanupAuthority::OwnedWhenSafe
        || resource.creation_operation_id.as_deref() != Some(identity.creation_operation_id.as_str())
        || run.setup.as_ref().and_then(|setup| setup.operation_id.as_deref()) != Some(identity.creation_operation_id.as_str())
        || !resource.consumers.is_empty() || registered.consumers.iter().any(|consumer| OrchestrationService::consumer_unresolved(state, consumer))
        || OrchestrationService::known_writers_active(state, registered)
        || !retirement::blockers(state, run).is_empty() { return None; }
    let retirement = run.retirement.as_ref()?;
    if !matches!(retirement.state, RetirementState::Retired { .. })
        || !retirement::authority_matches(state, run, retirement) { return None; }
    let attachment = run.attachment.as_ref().filter(|attachment| attachment.state == AttachmentState::Revoked)?;
    let scope = attachment.process_scope.as_ref()?;
    let checkout = run.checkout.as_ref()?;
    if checkout.key != identity.checkout_key || checkout.path != identity.checkout_path
        || checkout.directory_identity.device != identity.checkout_device || checkout.directory_identity.inode != identity.checkout_inode
        || checkout.git.as_ref().is_none_or(|git| git.common_dir != identity.common_dir)
        || attachment.process.as_ref() != Some(&scope.member) { return None; }
    let claim = state.checkout_claims.iter().find(|claim| Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()
        && claim.state == claims::ClaimState::Released && claim.holder.is_none() && &claim.checkout == checkout)?;
    let operation_id = claim.transition_operation_id.as_ref()?;
    let released = state.operations.iter().any(|operation| &operation.operation_id == operation_id
        && operation.kind == DurableOperationKind::ScopeRelease && operation.run_id.as_deref() == Some(run.run_id.as_str())
        && operation.root_id == run.root_id && operation.actor_run_id.is_none()
        && operation.principal_key.strip_prefix("dispatcher-scope:") == Some(run.run_id.as_str())
        && operation.effect_receipts.iter().any(|receipt| matches!(receipt, DurableEffectReceipt::ScopeRelease {
            unit, invocation_id, claim_id, outcome: ScopeReleaseOutcome::Released,
        } if unit == &scope.unit && invocation_id == &scope.invocation_id && claim_id == &claim.claim_id)));
    released.then_some(claim.revision)
}

/// A registration proof is made by the creation adapter, never deserialized
/// from a caller. Its request and run incarnation are rechecked under the lock.
#[derive(Debug, Clone)]
pub struct ResourceRegistrationFacts {
    request_digest: String,
    run_id: String,
    attempt: u32,
    setup_operation_id: Option<String>,
    identity: OwnedResourceIdentity,
}

pub fn reason(class: CleanupReasonClass, code: &str, message: impl Into<String>) -> CleanupReason {
    CleanupReason { class, code: code.into(), message: message.into() }
}

fn identity_digest(identity: &OwnedResourceIdentity) -> Result<String, InspectionError> {
    let bytes = serde_json::to_vec(identity)
        .map_err(|_| error("resource_identity_invalid", "Cannot encode resource identity"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn kind(identity: &OwnedResourceIdentity) -> OwnedResourceKind {
    match identity {
        OwnedResourceIdentity::WorkerTerminal { .. } => OwnedResourceKind::WorkerTerminal,
        OwnedResourceIdentity::Worktree { .. } => OwnedResourceKind::Worktree,
        OwnedResourceIdentity::Branch { .. } => OwnedResourceKind::Branch,
        OwnedResourceIdentity::Space { .. } => OwnedResourceKind::Space,
        OwnedResourceIdentity::Scratch { .. } => OwnedResourceKind::Scratch,
        OwnedResourceIdentity::FixtureSet { .. } => OwnedResourceKind::FixtureSet,
        OwnedResourceIdentity::AttemptDirectory { .. } => OwnedResourceKind::AttemptDirectory,
    }
}

fn creation_operation(identity: &OwnedResourceIdentity) -> &str {
    match identity {
        OwnedResourceIdentity::WorkerTerminal { identity } => &identity.launch_tag,
        OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::Space { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::Scratch { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::FixtureSet { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::AttemptDirectory { identity } => &identity.creation_operation_id,
    }
}

fn same_resource(left: &OwnedResourceIdentity, right: &OwnedResourceIdentity) -> bool {
    match (left, right) {
        (OwnedResourceIdentity::WorkerTerminal { identity: a }, OwnedResourceIdentity::WorkerTerminal { identity: b }) =>
            a.attachment_id == b.attachment_id && a.attachment_epoch == b.attachment_epoch && a.terminal_id == b.terminal_id,
        (OwnedResourceIdentity::Worktree { identity: a }, OwnedResourceIdentity::Worktree { identity: b }) =>
            !a.checkout_inode.is_empty() && a.machine_id == b.machine_id && a.checkout_device == b.checkout_device && a.checkout_inode == b.checkout_inode,
        (OwnedResourceIdentity::Branch { identity: a }, OwnedResourceIdentity::Branch { identity: b }) =>
            !a.common_inode.is_empty() && a.machine_id == b.machine_id && a.common_device == b.common_device && a.common_inode == b.common_inode && a.branch_ref == b.branch_ref,
        (OwnedResourceIdentity::Space { identity: a }, OwnedResourceIdentity::Space { identity: b }) =>
            a.endpoint_identity == b.endpoint_identity && a.session_id == b.session_id && a.workspace_id == b.workspace_id,
        (OwnedResourceIdentity::Scratch { identity: a }, OwnedResourceIdentity::Scratch { identity: b }) =>
            a.machine_id == b.machine_id && a.device == b.device && a.inode == b.inode,
        (OwnedResourceIdentity::FixtureSet { identity: a }, OwnedResourceIdentity::FixtureSet { identity: b }) =>
            a.filesystem.machine_id == b.filesystem.machine_id && a.filesystem.device == b.filesystem.device && a.filesystem.inode == b.filesystem.inode,
        (OwnedResourceIdentity::AttemptDirectory { identity: a }, OwnedResourceIdentity::AttemptDirectory { identity: b }) =>
            a.root == b.root && a.device == b.device && a.inode == b.inode,
        _ => false,
    }
}

fn insert_resource(resources: &mut Vec<OwnedResourceRecord>, run: &Run, identity: OwnedResourceIdentity) -> Result<usize, InspectionError> {
    if let Some((index, existing)) = resources.iter().enumerate().find(|(_, r)| r.identity == identity) {
        if existing.run_id != run.run_id || existing.root_id != run.root_id {
            return Err(error("resource_ownership_conflict", "Creation receipt already belongs to a different run"));
        }
        return Ok(index);
    }
    if resources.iter().any(|r| same_resource(&r.identity, &identity)) {
        return Err(error("resource_ownership_conflict", "Resource has conflicting creation provenance"));
    }
    let at = now();
    let consumers = run.attachment.as_ref().map(|attachment| AttachmentReference {
        run_id: run.run_id.clone(), attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch,
    }).into_iter().collect();
    let owned = run.cleanup_policy == Some(WorkerCleanupPolicy::OwnedWhenSafe);
    let record = OwnedResourceRecord {
        resource_id: identity_digest(&identity)?, root_id: run.root_id.clone(), run_id: run.run_id.clone(),
        kind: kind(&identity), ownership: if owned { ResourceOwnership::Owned } else { ResourceOwnership::Borrowed },
        creation_operation_id: Some(creation_operation(&identity).into()), identity,
        authority: if owned { CleanupAuthority::OwnedWhenSafe } else { CleanupAuthority::AuthorityUnproven },
        consumers, integration_result_id: None,
        cleanup: CleanupItem { operation_id: None, state: CleanupState::Pending, reason: None, effects: Vec::new(), updated_at: at },
    };
    let index = resources.len();
    resources.push(record);
    Ok(index)
}

fn authorize(ctx: &MutationCtx<'_>, root_id: &str, run_id: Option<&str>) -> Result<(), InspectionError> {
    let root = scoped_index(ctx.state, ctx.session_id, root_id)?;
    if ctx.state.runs[root].root_id != root_id || ctx.state.runs[root].parent_run_id.is_some() {
        return Err(error("root_not_found", "Cleanup scope must name an actual root"));
    }
    if let Some(run_id) = run_id {
        let index = scoped_index(ctx.state, ctx.session_id, run_id)?;
        if ctx.state.runs[index].root_id != root_id {
            return Err(error("actor_forbidden", "Cleanup target belongs to a different root"));
        }
    }
    match ctx.actor {
        Actor::Operator(_) => Ok(()),
        Actor::Agent(agent) => {
            let index = required_caller(ctx.caller)?;
            let credential = ctx.state.credentials.iter().find(|c| c.credential_id == agent.auth.credential_id)
                .ok_or_else(|| error("job_auth_invalid", "Admitted cleanup principal is missing"))?;
            let caller = &ctx.state.runs[index];
            let role = credential.role;
            if (role == JobRole::Root && caller.run_id == root_id)
                || (role == JobRole::Worker && run_id == Some(caller.run_id.as_str()) && caller.root_id == root_id) {
                Ok(())
            } else { Err(error("actor_forbidden", "Only the root or exact owning worker can request cleanup")) }
        }
    }
}

pub(in crate::orchestration) fn authorize_replay(ctx: &MutationCtx<'_>, action: &OrchestrationAction) -> Result<(), InspectionError> {
    match action {
        OrchestrationAction::CleanupFinish { request } => authorize(ctx, &request.root_id, request.run_id.as_deref()),
        OrchestrationAction::ResourceRegister { request } => {
            let run = &ctx.state.runs[scoped_index(ctx.state, ctx.session_id, &request.run_id)?];
            authorize(ctx, &run.root_id, Some(&run.run_id))
        }
        _ => Err(error("invalid_action", "Expected a cleanup operation")),
    }
}

pub(in crate::orchestration) fn admit_caller(state: &OrchestrationState, caller: &AgentCaller, request: &CleanupFinish) -> Result<usize, InspectionError> {
    let job = credentials::validate_cleanup_job_auth(state, &caller.auth)?;
    let run = &state.runs[job.run_index];
    credentials::validate_current_assignment(state, caller, &job)?;
    if (job.role == JobRole::Root && request.root_id != run.run_id)
        || (job.role == JobRole::Worker && (request.root_id != run.root_id || request.run_id.as_deref() != Some(run.run_id.as_str()))) {
        return Err(error("actor_forbidden", "Cleanup requires the exact authenticated root or owning worker"));
    }
    if let Some(target) = &request.run_id {
        if state.runs[scoped_index(state, &run.session_id, target)?].root_id != request.root_id {
            return Err(error("actor_forbidden", "Cleanup target is outside the root"));
        }
    }
    Ok(job.run_index)
}

pub(in crate::orchestration) fn apply(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    match &action {
        OrchestrationAction::ResourceRegister { request } => {
            let index = scoped_index(ctx.state, ctx.session_id, &request.run_id)?;
            let root_id = ctx.state.runs[index].root_id.clone();
            authorize(ctx, &root_id, Some(&request.run_id))?;
            if let Some(result) = resume::replay(ctx, &request.operation_id, DurableOperationKind::ResourceRegister, &action)? {
                return Ok(Applied::Document(result));
            }
            let proof = ctx.resolved.and_then(|r| r.resource_registration.as_ref())
                .ok_or_else(|| error("resource_creation_unverified", "Registration requires the actual creation adapter receipt"))?;
            let run = &ctx.state.runs[index];
            if proof.request_digest != resume::request_digest(&action, ctx.session_id)? || proof.run_id != run.run_id
                || proof.attempt != run.attempt || proof.setup_operation_id != run.setup.as_ref().and_then(|s| s.operation_id.clone())
                || proof.identity != request.resource {
                return Err(error("resource_creation_conflict", "Creation proof no longer matches this request and run"));
            }
            let resource = insert_resource(&mut ctx.state.owned_resources, run, request.resource.clone())?;
            let result = OrchestrationActionResult::ResourceRegistered { resource: ctx.state.owned_resources[resource].clone(), duplicate: false };
            resume::record(ctx, request.operation_id.clone(), DurableOperationKind::ResourceRegister, &action, root_id, result.clone(), Vec::new())?;
            Ok(Applied::Machine(result))
        }
        OrchestrationAction::CleanupFinish { request } => {
            authorize(ctx, &request.root_id, request.run_id.as_deref())?;
            if let Some(result) = resume::replay(ctx, &request.operation_id, DurableOperationKind::CleanupFinish, &action)? {
                return Ok(Applied::Document(result));
            }
            let own_worker = matches!(ctx.actor, Actor::Agent(agent) if credentials::validate_cleanup_job_auth(ctx.state, &agent.auth)?.role == JobRole::Worker);
            let scoped: Vec<String> = ctx.state.runs.iter().filter(|r| r.root_id == request.root_id && request.run_id.as_deref().is_none_or(|id|
                r.run_id == id || (!own_worker && is_ancestor(ctx.state, id, &r.run_id))))
                .map(|r| r.run_id.clone()).collect();
            for resource in &mut ctx.state.owned_resources {
                if scoped.contains(&resource.run_id) && resource.cleanup.state != CleanupState::Removed
                    && resource.ownership == ResourceOwnership::Owned && resource.authority == CleanupAuthority::OwnedWhenSafe {
                    resource.cleanup.operation_id = Some(request.operation_id.clone());
                    // This is scheduling, not extra authority. Unknown intents
                    // remain unknown and must be inspected before any effect.
                    if !resource.cleanup.effects.iter().any(|e| matches!(e.state, CleanupEffectState::Intent | CleanupEffectState::Unknown)) {
                        resource.cleanup.state = CleanupState::Pending;
                    }
                    resource.cleanup.updated_at = now();
                }
            }
            let items = ctx.state.owned_resources.iter().filter(|r| scoped.contains(&r.run_id)).cloned().collect();
            let result = OrchestrationActionResult::CleanupFinished { cleanup: finish_result(&request.operation_id, items, false) };
            resume::record(ctx, request.operation_id.clone(), DurableOperationKind::CleanupFinish, &action, request.root_id.clone(), result.clone(), Vec::new())?;
            Ok(Applied::Machine(result))
        }
        _ => Err(error("invalid_action", "Not an owned-resource operation")),
    }
}

pub(crate) fn finish_result(operation_id: &str, items: Vec<OwnedResourceRecord>, duplicate: bool) -> CleanupFinishResult {
    let (removed_count, waiting_count, kept_count, unknown_count) = counts(items.iter().map(|r| r.cleanup.state));
    CleanupFinishResult { operation_id: operation_id.into(), items, removed_count, waiting_count, kept_count, unknown_count, duplicate }
}

fn counts(states: impl Iterator<Item = CleanupState>) -> (u32, u32, u32, u32) {
    let (mut removed, mut waiting, mut kept, mut unknown) = (0, 0, 0, 0);
    for state in states {
        match state {
            CleanupState::Removed => removed += 1,
            CleanupState::Kept => kept += 1,
            CleanupState::Unknown => unknown += 1,
            _ => waiting += 1,
        }
    }
    (removed, waiting, kept, unknown)
}

pub(crate) fn summaries(state: &OrchestrationState, session_id: &str) -> Vec<RunCleanupSummary> {
    state.runs.iter().filter(|run| run.session_id == session_id).map(|run| {
        let items: Vec<_> = state.owned_resources.iter().filter(|r| r.run_id == run.run_id).map(|r| {
            let mut cleanup = r.cleanup.clone();
            // Diagnostic effect evidence may contain private filesystem paths.
            for effect in &mut cleanup.effects { effect.evidence = None; }
            if let Some(why) = &mut cleanup.reason {
                why.message = format!("Resource cleanup status: {}.", why.code);
            }
            CleanupResourceSummary { resource_id: r.resource_id.clone(), kind: r.kind, ownership: r.ownership, cleanup }
        }).collect();
        let (removed_count, waiting_count, kept_count, unknown_count) = counts(items.iter().map(|r| r.cleanup.state));
        RunCleanupSummary { run_id: run.run_id.clone(), items, removed_count, waiting_count, kept_count, unknown_count }
    }).collect()
}

pub(crate) fn restoration_summary(state: &OrchestrationState, session_id: &str) -> RestorationSummary {
    // Progress covers recovery operations, not every retained attachment.
    // Blocked recovery remains actionable on its run rather than spinning here.
    let (mut reattached, mut total_attachments) = (0, 0);
    for run in state.runs.iter().filter(|run| run.session_id == session_id
        && !matches!(run.stage, RunStage::Closed | RunStage::Reported)
        && run.result.is_none()) {
        match run.resume.as_ref().map(|resume| resume.phase) {
            Some(ResumePhase::Reattached | ResumePhase::Connected) => {
                reattached += 1;
                total_attachments += 1;
            }
            Some(ResumePhase::Reconciling | ResumePhase::Launching) => total_attachments += 1,
            Some(ResumePhase::Blocked) | None => {}
        }
    }
    // Registration alone is not cleanup in progress. Effects require a closed
    // run; waiting or uncertain resources retain their per-run diagnostics.
    let cleanup_pending = state.owned_resources.iter().filter(|resource| {
        resource.ownership == ResourceOwnership::Owned
            && resource.authority == CleanupAuthority::OwnedWhenSafe
            && matches!(resource.cleanup.state, CleanupState::Pending | CleanupState::Running | CleanupState::Reconciling)
            && state.runs.iter().any(|run| run.session_id == session_id
                && run.run_id == resource.run_id && run.stage == RunStage::Closed)
    }).count() as u32;
    RestorationSummary { reattached, total_attachments, cleanup_pending }
}

/// Returns only event-relevant cleanup candidates; attempts/resources are never
/// grouped by canonical task, so superseded output cannot inherit successor proof.
fn cleanup_candidates(state: &OrchestrationState) -> Vec<OwnedResourceRecord> {
    state.owned_resources.iter().filter(|resource| resource.cleanup.state != CleanupState::Removed
        && state.runs.iter().any(|run| run.run_id == resource.run_id && run.stage == RunStage::Closed))
        .cloned().collect()
}

fn setup_resources_captured(resources: &[OwnedResourceRecord], run: &Run) -> bool {
    let Some(setup) = &run.setup else { return false };
    let raw = setup.operation_id.as_deref().is_some_and(|operation|
        resources.iter().any(|resource| resource.run_id == run.run_id
            && resource.creation_operation_id.as_deref() == Some(operation)));
    let Some(workspace_id) = setup.workspace_id.as_deref() else { return raw };
    let bound = resources.iter().any(|resource| resource.run_id == run.run_id
        && matches!(&resource.identity, OwnedResourceIdentity::Space { identity }
            if identity.workspace_id == workspace_id && identity.session_id == run.session_id
                && identity.checkout_path == setup.checkout_path
                && run.dispatch.as_ref().and_then(|dispatch| dispatch.move_operation_id.as_ref())
                    .is_none_or(|operation| operation == &identity.creation_operation_id)));
    let borrowed_space_only = setup.operation_id.is_none()
        && run.dispatch.as_ref().and_then(|dispatch| dispatch.move_operation_id.as_ref()).is_some()
        && run.checkout.as_ref().is_some_and(|checkout| checkout.path == setup.checkout_path);
    bound && (raw || borrowed_space_only || matches!(setup.ownership,
        Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory)))
}

fn attempt_directory_released(state: &OrchestrationState, run: &Run) -> bool {
    run.stage == RunStage::Closed && run.close_reason == Some(CloseReason::Accepted)
        && run.result.as_ref().is_some_and(|result| result.outcome == Some(ReportOutcome::Succeeded))
        && run.retirement.as_ref().is_some_and(|record| matches!(record.state, RetirementState::Retired { .. })
            && retirement::identity_matches(run, record))
        && state.checkout_claims.iter().any(|claim| Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()
            && claim.state == claims::ClaimState::Released && run.checkout.as_ref() == Some(&claim.checkout))
}

impl OrchestrationService {
    fn owned_consumer_proof<'a>(&'a self, resource: &'a OwnedResourceRecord, run: &'a Run, merged: Option<&'a merged::MergedProof>) -> Result<Option<OwnedConsumerProof<'a>>, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        Ok(released_consumer_revision(&state, resource, run).filter(|_| merged.is_none_or(|proof| proof.matches_state(&state, run)))
            .map(|claim_revision| OwnedConsumerProof { service: self, resource, run, claim_revision, merged }))
    }

    fn validate_consumer_release(&self, resource: &OwnedResourceRecord, run: &Run, claim_revision: u64, merged: Option<&merged::MergedProof>) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        if released_consumer_revision(&state, resource, run) != Some(claim_revision)
            || merged.is_some_and(|proof| !proof.matches_state(&state, run)) {
            return Err(error("consumer_proof_missing", "Exact released claim revision, holder epoch, creation identity or owned scope journal is no longer current"));
        }
        Ok(())
    }

    /// The scheduler evaluates this queue on acceptance, closure, pane/owner and
    /// relevant Git-ref events. No lease or elapsed time grants cleanup authority.
    pub fn cleanup_queue(&self) -> Result<Vec<OwnedResourceRecord>, InspectionError> {
        Ok(cleanup_candidates(&self.store.lock()?.read()?))
    }

    /// Git common directories to register before reading/evaluating the queue.
    /// Removed resources are omitted so the shared notification owner can unwatch.
    pub fn cleanup_ref_watch_paths(&self) -> Result<Vec<std::path::PathBuf>, InspectionError> {
        let state = self.store.lock()?.read()?;
        let mut paths = std::collections::BTreeSet::new();
        for resource in state.owned_resources.iter().filter(|resource|
            resource.cleanup.state != CleanupState::Removed
                && state.runs.iter().any(|run| run.run_id == resource.run_id && run.stage == RunStage::Closed)) {
            if let OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } = &resource.identity {
                if !identity.common_dir.is_empty() { paths.insert(std::path::PathBuf::from(&identity.common_dir)); }
            }
        }
        for run in state.runs.iter().filter(|run| run.stage == RunStage::Closed
            && run.close_reason == Some(CloseReason::Accepted) && !git_integration_eligible(run)
            && state.owned_resources.iter().any(|resource| resource.run_id == run.run_id
                && matches!(resource.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch)
                && resource.cleanup.state != CleanupState::Removed)) {
            for related in state.runs.iter().filter(|related| related.root_id == run.root_id
                && related.session_id == run.session_id) {
                if let Some(git) = related.checkout.as_ref().and_then(|checkout| checkout.git.as_ref()) {
                    paths.insert(std::path::PathBuf::from(&git.common_dir));
                    if related.run_id == run.root_id {
                        paths.insert(std::path::PathBuf::from(&git.worktree_git_dir));
                    }
                }
            }
        }
        // Pending schedules still need integration/input wakes after predecessor
        // resources have been removed. Project identity survives private root cwd.
        let scheduled_roots: std::collections::BTreeSet<_> = state.task_ownership.iter()
            .filter(|ownership| ownership.schedule.is_some()).map(|ownership| ownership.root_id.as_str()).collect();
        for run in state.runs.iter().filter(|run| scheduled_roots.contains(run.root_id.as_str())) {
            if let Some(ProjectSource::Git { common_dir }) = run.project.as_ref().map(|project| &project.source) {
                if !common_dir.is_empty() && !paths.contains(std::path::Path::new(common_dir)) {
                    paths.insert(std::path::PathBuf::from(common_dir));
                }
            }
        }
        Ok(paths.into_iter().collect())
    }

    pub fn cleanup_job_run(&self, caller: &AgentCaller, request: &CleanupFinish) -> Result<Run, InspectionError> {
        let state = self.store.lock()?.read()?;
        let job = credentials::validate_cleanup_job_auth(&state, &caller.auth)?;
        credentials::validate_current_assignment(&state, caller, &job)?;
        let run = &state.runs[job.run_index];
        if (job.role == JobRole::Root && request.root_id != run.run_id)
            || (job.role == JobRole::Worker && (request.root_id != run.root_id || request.run_id.as_deref() != Some(run.run_id.as_str()))) {
            return Err(error("actor_forbidden", "Cleanup bootstrap is scoped to the authenticated root or own worker"));
        }
        if let Some(target) = &request.run_id {
            if state.runs[scoped_index(&state, &run.session_id, target)?].root_id != request.root_id {
                return Err(error("actor_forbidden", "Cleanup target is outside the authenticated root"));
            }
        }
        Ok(run.clone())
    }

    pub async fn resolve_resource_registration(&self, projects: &ProjectService, actor: &Actor, session_id: &str, action: &OrchestrationAction) -> Result<ResourceRegistrationFacts, InspectionError> {
        let OrchestrationAction::ResourceRegister { request } = action else {
            return Err(error("invalid_action", "Expected ResourceRegister"));
        };
        let run = {
            let state = self.store.lock()?.read()?;
            let target = scoped_index(&state, session_id, &request.run_id)?;
            match actor {
                Actor::Operator(_) => {}
                Actor::Agent(agent) => {
                    let job = credentials::validate_caller(&state, agent)?;
                    let caller = &state.runs[job.run_index];
                    if caller.session_id != session_id
                        || !((job.role == JobRole::Root && state.runs[target].root_id == caller.run_id)
                            || (job.role == JobRole::Worker && target == job.run_index)) {
                        return Err(error("actor_forbidden", "Registration is scoped to the root or exact owning worker"));
                    }
                }
            }
            state.runs[target].clone()
        };
        match &request.resource {
            OwnedResourceIdentity::FixtureSet { .. } | OwnedResourceIdentity::Scratch { .. } =>
                super::fixture_resources::validate_registration(&request.resource, &run).await?,
            OwnedResourceIdentity::Worktree { .. } | OwnedResourceIdentity::Branch { .. } | OwnedResourceIdentity::AttemptDirectory { .. } => {
                if run.setup.as_ref().and_then(|s| s.operation_id.as_deref()) != Some(creation_operation(&request.resource)) {
                    return Err(error("resource_creation_conflict", "Resource was not created by this run setup"));
                }
                projects.validate_owned_resource(&request.resource).await?;
            }
            OwnedResourceIdentity::Space { .. } => {
                if !projects.owned_resources_for_run(&run).await?.iter().any(|identity| identity == &request.resource) {
                    return Err(error("resource_creation_conflict", "Space is not associated with this exact run's setup creation receipt"));
                }
                projects.validate_owned_resource(&request.resource).await?;
            }
            OwnedResourceIdentity::WorkerTerminal { identity } => {
                if retirement::identity_for(&run).as_ref() != Some(identity) {
                    return Err(error("resource_creation_conflict", "Terminal does not match this exact launch receipt"));
                }
            }
        }
        Ok(ResourceRegistrationFacts { request_digest: resume::request_digest(action, session_id)?, run_id: run.run_id,
            attempt: run.attempt, setup_operation_id: run.setup.and_then(|s| s.operation_id), identity: request.resource.clone() })
    }

    fn terminal_capture_identity(state: &OrchestrationState, run: &Run) -> Option<RetirementIdentity> {
        if run.stage == RunStage::Closed {
            // Closure pins the actual retirement receipt; never recapture a
            // mutable attachment or retrofit an already captured launch.
            let record = run.retirement.as_ref()?;
            return retirement::authority_matches(state, run, record)
                .then(|| record.identity.clone()).flatten();
        }
        // Starting may still be the latch wrapper, with the same PID/start
        // that will later exec the harness. It is not its creation fingerprint.
        if !has_current_binding(state, run) { return None; }
        let identity = retirement::identity_for(run)?;
        let shell = identity.shell.as_ref()?;
        let pid = i32::try_from(shell.process.pid).ok()?;
        if crate::process_identity::executable_identity(pid, shell.process.start_ticks).ok().as_ref() != Some(shell) {
            return None;
        }
        Some(identity)
    }

    async fn terminal_capture_observed(herdr: &dyn super::herdr::OrchestrationHerdr,
        run: &Run, identity: &RetirementIdentity) -> bool {
        let Ok(runtime) = herdr.runtime(&identity.session_id).await else { return false };
        if retirement::server_fence(identity, &runtime).is_err() { return false; }
        let same_terminal = runtime.panes.iter().filter(|pane|
            pane.terminal_id.as_deref() == Some(identity.terminal_id.as_str())).count();
        let Some(pane) = runtime.panes.iter().find(|pane| pane.pane_id == identity.pane_id) else {
            // Only the existing immutable closure pin can join fresh absence.
            return run.stage == RunStage::Closed && same_terminal == 0;
        };
        if run.stage == RunStage::Closed || same_terminal != 1
            || pane.terminal_id.as_deref() != Some(identity.terminal_id.as_str())
            || pane.workspace_id != identity.workspace_id || pane.tab_id != identity.tab_id
            || pane.tab_label != identity.launch_tag
            || pane.agent_kind.as_ref().is_some_and(|kind| kind != &run.launch_config.harness_kind)
            || pane.native_session_id.as_ref().zip(identity.conversation.as_ref())
                .is_some_and(|(native, conversation)| native != &conversation.session_id) {
            return false;
        }
        let (Some(process), Some(shell)) = (&identity.process, &identity.shell) else { return false };
        if identity.launch_mode != Some(AttachmentLaunchMode::DirectProcess) || shell.process != *process { return false; }
        let Ok(info) = herdr.pane_process_info(&identity.session_id, &identity.endpoint_identity, &identity.pane_id).await
            else { return false };
        // Capture does not authorize retirement: foreground children, active
        // work and shared surfaces remain protected by the separate close gates.
        info.pane_id == identity.pane_id && info.shell_pid == Some(process.pid)
            && info.shell_identity.as_ref() == Some(shell)
            && info.processes.iter().any(|(pid, _)| *pid == process.pid)
    }

    /// Startup/reconnect capture reads original creation receipts, never adopts
    /// a path merely because it currently exists. The same proof is one record.
    pub async fn capture_owned_resources(&self, projects: &ProjectService,
        herdr: &dyn super::herdr::OrchestrationHerdr) -> Result<(), InspectionError> {
        let saved = self.store.lock()?.read()?;
        for run in saved.runs.iter().filter(|run| run.kind == RunKind::Worker) {
            let setup_operation = run.setup.as_ref().and_then(|s| s.operation_id.as_deref());
            let move_operation = run.dispatch.as_ref().and_then(|dispatch| dispatch.move_operation_id.as_ref());
            let captured = setup_resources_captured(&saved.owned_resources, run);
            let (mut identities, limitation) = if captured || (setup_operation.is_none() && move_operation.is_none()) {
                (Vec::new(), false)
            } else {
                // Association/transport failures are not missing Git pins.
                // Only the original creation receipt can attest that limitation.
                let identities = projects.owned_resources_for_run(run).await?;
                let limitation = projects.owned_creation_limitation_for_run(run).await?.is_some();
                (identities, limitation)
            };
            let terminal_captured = run.attachment.as_ref().is_some_and(|attachment| saved.owned_resources.iter().any(|resource|
                matches!(&resource.identity, OwnedResourceIdentity::WorkerTerminal { identity }
                    if resource.run_id == run.run_id && identity.attachment_id.as_deref() == Some(attachment.attachment_id.as_str())
                        && identity.attachment_epoch == Some(attachment.epoch))));
            if !terminal_captured {
                if let Some(identity) = Self::terminal_capture_identity(&saved, run) {
                    if Self::terminal_capture_observed(herdr, run, &identity).await {
                        identities.push(OwnedResourceIdentity::WorkerTerminal { identity });
                    }
                }
            }
            if identities.is_empty() && !limitation { continue; }
            let locked = self.store.lock()?;
            let mut state = locked.read()?;
            let index = run_index(&state, &run.run_id)?;
            let fresh = &state.runs[index];
            if fresh.attempt != run.attempt || fresh.setup.as_ref().and_then(|s| s.operation_id.as_ref()) != run.setup.as_ref().and_then(|s| s.operation_id.as_ref())
                || fresh.setup.as_ref().and_then(|s| s.workspace_id.as_ref()) != run.setup.as_ref().and_then(|s| s.workspace_id.as_ref())
                || fresh.dispatch.as_ref().and_then(|dispatch| dispatch.move_operation_id.as_ref()) != run.dispatch.as_ref().and_then(|dispatch| dispatch.move_operation_id.as_ref())
                || fresh.attachment.as_ref().map(|attachment| (&attachment.attachment_id, attachment.epoch))
                    != run.attachment.as_ref().map(|attachment| (&attachment.attachment_id, attachment.epoch)) { continue; }
            let before = state.owned_resources.len();
            let mut conflict = false;
            for identity in identities {
                if let OwnedResourceIdentity::WorkerTerminal { identity: terminal } = &identity {
                    if Self::terminal_capture_identity(&state, fresh).as_ref() != Some(terminal) { continue; }
                }
                let conflicts = state.owned_resources.iter().any(|resource| same_resource(&resource.identity, &identity)
                    && (resource.identity != identity || resource.run_id != fresh.run_id || resource.root_id != fresh.root_id));
                if conflicts {
                    for resource in &mut state.owned_resources {
                        if same_resource(&resource.identity, &identity) && resource.ownership != ResourceOwnership::Unknown {
                            resource.ownership = ResourceOwnership::Unknown;
                            resource.authority = CleanupAuthority::AuthorityUnproven;
                            resource.cleanup.state = CleanupState::Unknown;
                            resource.cleanup.reason = Some(reason(CleanupReasonClass::Authority, "creation_provenance_conflict", "Conflicting creation receipts protect this resource"));
                            resource.cleanup.updated_at = now();
                            conflict = true;
                        }
                    }
                } else {
                    insert_resource(&mut state.owned_resources, fresh, identity)?;
                }
            }
            if limitation { Self::record_unknown_git(&mut state.owned_resources, fresh)?; }
            if state.owned_resources.len() != before || conflict {
                sync_retirement_receipts(&mut state, index);
                let revision = locked.save(&mut state)?;
                self.revision.send_replace(revision);
            }
        }
        Ok(())
    }

    fn record_unknown_git(resources: &mut Vec<OwnedResourceRecord>, run: &Run) -> Result<(), InspectionError> {
        let Some(setup) = run.setup.as_ref().filter(|s| s.operation_id.is_some() && !s.checkout_path.is_empty()) else { return Ok(()) };
        if !matches!(run.project.as_ref().map(|project| &project.source), Some(ProjectSource::Git { .. }))
            && run.checkout.as_ref().is_none_or(|checkout| checkout.git.is_none()) { return Ok(()) }
        let identity = OwnedGitIdentity {
            creation_operation_id: setup.operation_id.as_ref().expect("filtered operation").clone(),
            machine_id: String::new(), repository_key: String::new(), repository_root: String::new(),
            repository_device: String::new(), repository_inode: String::new(), common_dir: String::new(),
            common_device: String::new(), common_inode: String::new(), checkout_path: setup.checkout_path.clone(),
            checkout_device: String::new(), checkout_inode: String::new(), checkout_key: String::new(),
            branch_ref: setup.branch.as_ref().map(|branch| if branch.starts_with("refs/heads/") { branch.clone() } else { format!("refs/heads/{branch}") }),
            creation_tip: setup.base_commit.clone().unwrap_or_default(),
        };
        let mut identities = Vec::with_capacity(2);
        if identity.branch_ref.is_some() { identities.push(OwnedResourceIdentity::Branch { identity: identity.clone() }); }
        identities.push(OwnedResourceIdentity::Worktree { identity });
        for identity in identities {
            if resources.iter().any(|resource| resource.identity == identity) { continue; }
            let index = insert_resource(resources, run, identity)?;
            let resource = &mut resources[index];
            resource.ownership = ResourceOwnership::Unknown;
            resource.authority = CleanupAuthority::AuthorityUnproven;
            resource.cleanup.state = CleanupState::Unknown;
            resource.cleanup.reason = Some(reason(CleanupReasonClass::Authority, "creation_identity_unavailable", "Original Git creation pins are unavailable; ownership cannot be granted retroactively"));
        }
        Ok(())
    }

    fn writer_quiescent(state: &OrchestrationState, run: &Run, attachment_id: &str, epoch: u64) -> bool {
        let Some(attachment) = run.attachment.as_ref().filter(|a| a.attachment_id == attachment_id && a.epoch == epoch)
            else { return false };
        if run.stage != RunStage::Closed || !retirement::blockers(state, run).is_empty() { return false; }
        // All old-boot consumers are absent only on the same verified machine.
        // A main-process exit or pane close alone cannot attest detached writers.
        let reboot = attachment.machine_id.as_ref().is_some_and(|machine|
            crate::process_identity::machine_id().as_ref() == Some(machine))
            && attachment.process.as_ref().or_else(|| attachment.shell.as_ref().map(|shell| &shell.process))
                .and_then(|process| process.kernel_boot_id.as_ref()).is_some_and(|boot|
                    crate::process_identity::kernel_boot_id().as_ref().is_some_and(|current| current != boot));
        reboot || run.checkout_claim_id.as_ref().is_some_and(|id|
            state.checkout_claims.iter().any(|claim| &claim.claim_id == id && claim.state == claims::ClaimState::Released))
    }

    fn consumer_unresolved(state: &OrchestrationState, consumer: &AttachmentReference) -> bool {
        state.runs.iter().find(|r| r.run_id == consumer.run_id)
            .is_none_or(|run| !Self::writer_quiescent(state, run, &consumer.attachment_id, consumer.attachment_epoch))
    }

    fn consumers_unresolved(state: &OrchestrationState, resource: &OwnedResourceRecord) -> Vec<AttachmentReference> {
        resource.consumers.iter().filter(|consumer| Self::consumer_unresolved(state, consumer)).cloned().collect()
    }

    fn known_writers_active(state: &OrchestrationState, resource: &OwnedResourceRecord) -> bool {
        let (path, device, inode) = match &resource.identity {
            OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } =>
                (&identity.checkout_path, &identity.checkout_device, &identity.checkout_inode),
            OwnedResourceIdentity::AttemptDirectory { identity } => (&identity.root, &identity.device, &identity.inode),
            _ => return false,
        };
        let overlaps = |checkout: &CheckoutIdentity|
            (checkout.directory_identity.device == *device && checkout.directory_identity.inode == *inode)
                || claims::paths_overlap(&checkout.path, path);
        state.checkout_claims.iter().any(|claim| claim.state != claims::ClaimState::Released && overlaps(&claim.checkout)
            && claim.holder.as_ref().is_none_or(|holder| Self::consumer_unresolved(state, holder)))
            || state.runs.iter().any(|run| run.checkout.as_ref().is_some_and(overlaps)
                && run.attachment.as_ref().is_some_and(|attachment|
                    !Self::writer_quiescent(state, run, &attachment.attachment_id, attachment.epoch)))
    }

    /// Called by the existing owner scheduler; each resource is independently
    /// evaluated. A missing Space is not consulted for Git-only removal.
    pub async fn cleanup_owned_resources(&self, projects: &ProjectService) -> Result<(), InspectionError> {
        self.reconcile_accepted_integrations().await?;
        let queue = self.cleanup_queue()?;
        for queued in queue {
            if queued.cleanup.state == CleanupState::Removed { continue; }
            let Some(_lease) = self.execution_lease(&queued.run_id)? else { continue };
            if matches!(queued.kind, OwnedResourceKind::FixtureSet | OwnedResourceKind::Scratch)
                && queued.cleanup.effects.iter().any(|effect| matches!(effect.state, CleanupEffectState::Intent | CleanupEffectState::Unknown)) {
                if let Ok(receipts) = super::fixture_resources::effect_receipts(&queued).await {
                    self.record_constituent_receipts(&queued, receipts)?;
                }
            }
            let (resource, run, blocked, drained, directory_released, state) = {
                let state = self.store.lock()?.read()?;
                let Some(resource) = state.owned_resources.iter().find(|r| r.resource_id == queued.resource_id) else { continue };
                let run = &state.runs[run_index(&state, &resource.run_id)?];
                let blocked = retirement::blockers(&state, run);
                let mut observed = resource.clone();
                observed.consumers = Self::consumers_unresolved(&state, resource);
                let writers = Self::known_writers_active(&state, resource);
                let drained = run.attachment.as_ref().is_some_and(|a| Self::writer_quiescent(&state, run, &a.attachment_id, a.epoch));
                let directory_released = resource.kind != OwnedResourceKind::AttemptDirectory || attempt_directory_released(&state, run);
                (observed, run.clone(), !blocked.is_empty() || writers, drained, directory_released, state)
            };
            let needs_merged = matches!(resource.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch)
                && !git_integration_eligible(&run);
            let merged = if needs_merged { merged::merged_proof(&state, &run).await } else { Ok(None) };
            let merged = match merged {
                Ok(proof) => proof,
                Err(why) => {
                    self.save_evaluation(&resource, CleanupEvaluation::Unknown(
                        reason(CleanupReasonClass::Transient, &why.code, why.message)))?;
                    continue;
                }
            };
            let evaluation = if resource.ownership == ResourceOwnership::Unknown {
                CleanupEvaluation::Unknown(resource.cleanup.reason.clone().unwrap_or_else(|| reason(CleanupReasonClass::Authority, "ownership_unproven", "Unknown creation authority protects this resource")))
            } else if resource.ownership != ResourceOwnership::Owned || resource.authority != CleanupAuthority::OwnedWhenSafe {
                CleanupEvaluation::Kept(reason(CleanupReasonClass::Authority, "ownership_unproven", "Borrowed resources are retained"))
            } else if resource.kind == OwnedResourceKind::WorkerTerminal {
                terminal_evaluation(&resource, &run)
            } else if run.stage != RunStage::Closed {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "consumers_active", "Run or descendant consumers are still active"))
            } else if run.close_reason == Some(CloseReason::Superseded) && resource.kind == OwnedResourceKind::Space {
                match projects.inspect_owned_resource(&resource, &run, None, None).await {
                    Ok(CleanupEvaluation::Absent) => CleanupEvaluation::Absent,
                    _ => CleanupEvaluation::Kept(reason(CleanupReasonClass::UserWork, "superseded_space_preserved",
                        "No explicit Space teardown is authorized for replaced work; only fresh exact-incarnation absence can settle its bookkeeping")),
                }
            } else if run.close_reason == Some(CloseReason::Superseded)
                && resource.kind != OwnedResourceKind::WorkerTerminal {
                if !drained {
                    if let Some(why) = resource.cleanup.reason.as_ref().filter(|why|
                        why.code == "containment_unproven" || why.code == "checkout_consumer_present" || why.code.starts_with("scope_")) {
                        CleanupEvaluation::Kept(why.clone())
                    } else {
                        CleanupEvaluation::Kept(reason(CleanupReasonClass::UserWork, "superseded_work_preserved",
                            "Superseded attempt output and writer claims remain separate from its successor; preserve all old files and review this attempt independently"))
                    }
                } else {
                    CleanupEvaluation::Kept(reason(CleanupReasonClass::UserWork, "superseded_work_preserved",
                        "Superseded scope is drained, but all old work and outputs remain retained independently of its successor"))
                }
            } else if resource.kind == OwnedResourceKind::AttemptDirectory && run.close_reason != Some(CloseReason::Accepted) {
                CleanupEvaluation::Kept(reason(CleanupReasonClass::UserWork, "attempt_unaccepted",
                    "Superseded, cancelled and failed private attempt directories and their outputs are retained"))
            } else if resource.kind == OwnedResourceKind::AttemptDirectory && !directory_released {
                if let Some(why) = resource.cleanup.reason.as_ref().filter(|why|
                    why.code == "containment_unproven" || why.code == "checkout_consumer_present" || why.code.starts_with("scope_")) {
                    CleanupEvaluation::Kept(why.clone())
                } else {
                    CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "consumer_drain_unproven",
                        "Private attempt removal requires its exact checkout claim to be Released with consumer-exit proof and its own pane retired"))
                }
            } else if needs_merged && merged.is_none() {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "integration_pending",
                    "Record IntegrationRecord with required passed target checks, or merge the accepted head into the root's checked-out target"))
            } else if !drained && resource.cleanup.reason.as_ref().is_some_and(|why|
                why.code == "containment_unproven" || why.code == "checkout_consumer_present" || why.code.starts_with("scope_")) {
                CleanupEvaluation::Kept(resource.cleanup.reason.clone().expect("classified scope hold"))
            } else if blocked {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "consumers_active", "Known descendant or checkout writer consumers remain protected"))
            } else if matches!(resource.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch | OwnedResourceKind::Scratch | OwnedResourceKind::AttemptDirectory)
                && (!resource.consumers.is_empty() || !drained) {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "consumer_drain_unproven", "Known consumers have not supplied a proven sealed drain"))
            } else {
                let inspection = match resource.kind {
                    OwnedResourceKind::FixtureSet | OwnedResourceKind::Scratch => super::fixture_resources::inspect_owned_resource(&resource, &run).await,
                    OwnedResourceKind::AttemptDirectory => match projects.validate_cleanup_incarnation(&run).await {
                        Ok(()) => attempt_directory::inspect(self.base(), &resource, &run),
                        Err(why) => Err(why),
                    },
                    _ => {
                        let proof = self.owned_consumer_proof(&resource, &run, merged.as_ref())?;
                        projects.inspect_owned_resource(&resource, &run, proof.as_ref(), merged.as_ref()).await
                    },
                };
                inspection.unwrap_or_else(|why| if resource.kind == OwnedResourceKind::AttemptDirectory {
                    CleanupEvaluation::Unknown(reason(CleanupReasonClass::Authority, &why.code, why.message))
                } else { CleanupEvaluation::Unknown(reason(CleanupReasonClass::Transient, "inspection_unavailable", "Actual resource outcome could not be inspected")) })
            };
            let uncertain = resource.cleanup.effects.iter().any(|e| matches!(e.state, CleanupEffectState::Intent | CleanupEffectState::Unknown));
            // The supported fixture adapter's Ready result identifies untouched
            // constituents. It rechecks each fsynced intent and never repeats an
            // uncertain constituent; an old set-level receipt is not a veto.
            let uncertain_blocks_effect = uncertain && resource.kind != OwnedResourceKind::FixtureSet;
            if evaluation != CleanupEvaluation::Ready || uncertain_blocks_effect {
                let evaluated = if evaluation == CleanupEvaluation::Ready && uncertain_blocks_effect {
                    CleanupEvaluation::Unknown(reason(CleanupReasonClass::Reconcilable, "effect_outcome_unknown", "Prior effect remains present; inspection does not authorize repeating it"))
                } else { evaluation };
                self.save_evaluation(&resource, evaluated)?;
                continue;
            }
            let intent = self.begin_cleanup_effect(&resource, &run, merged.as_ref())?;
            let Some(intent) = intent else { continue };
            let outcome = match resource.kind {
                OwnedResourceKind::FixtureSet | OwnedResourceKind::Scratch => super::fixture_resources::apply_owned_resource(&intent, &run).await,
                OwnedResourceKind::AttemptDirectory => match projects.validate_cleanup_incarnation(&run).await {
                    Ok(()) => attempt_directory::apply(self.base(), &intent, &run),
                    Err(why) => Ok(CleanupEffectOutcome { state: CleanupEffectState::Refused, evidence: None,
                        reason: Some(reason(CleanupReasonClass::Authority, &why.code, why.message)), effects: Vec::new() }),
                },
                _ => {
                    let proof = self.owned_consumer_proof(&intent, &run, merged.as_ref())?;
                    projects.apply_owned_resource(&intent, &run, proof.as_ref(), merged.as_ref()).await
                },
            }.unwrap_or_else(|_| CleanupEffectOutcome { state: CleanupEffectState::Unknown, evidence: None,
                reason: Some(reason(CleanupReasonClass::Transient, "effect_outcome_unknown", "Effect did not return a confirmed outcome")), effects: Vec::new() });
            self.finish_cleanup_effect(&intent, outcome)?;
        }
        self.report_cleanup_holds()?;
        Ok(())
    }

    fn save_evaluation(&self, expected: &OwnedResourceRecord, evaluation: CleanupEvaluation) -> Result<(), InspectionError> {
        let (state, why) = match evaluation {
            CleanupEvaluation::Ready => return Ok(()),
            CleanupEvaluation::Absent => (CleanupState::Removed, None),
            CleanupEvaluation::Waiting(why) => (if why.code == "integration_pending" { CleanupState::WaitingIntegration } else { CleanupState::Waiting }, Some(why)),
            CleanupEvaluation::Kept(why) => (CleanupState::Kept, Some(why)),
            CleanupEvaluation::Unknown(why) => (CleanupState::Unknown, Some(why)),
        };
        let locked = self.store.lock()?;
        let mut stored = locked.read()?;
        let Some(resource) = stored.owned_resources.iter_mut().find(|r| r.resource_id == expected.resource_id) else { return Ok(()) };
        if resource.identity != expected.identity || resource.cleanup != expected.cleanup { return Ok(()) }
        if resource.cleanup.state == state && resource.cleanup.reason == why { return Ok(()) }
        resource.cleanup.state = state;
        resource.cleanup.reason = why;
        resource.cleanup.updated_at = now();
        if state == CleanupState::Removed {
            for effect in &mut resource.cleanup.effects {
                if matches!(effect.state, CleanupEffectState::Intent | CleanupEffectState::Unknown) {
                    effect.state = CleanupEffectState::Absent;
                    effect.updated_at = resource.cleanup.updated_at.clone();
                }
            }
        }
        let revision = locked.save(&mut stored)?;
        self.revision.send_replace(revision);
        Ok(())
    }

    fn begin_cleanup_effect(&self, expected: &OwnedResourceRecord, observed: &Run, merged: Option<&merged::MergedProof>) -> Result<Option<OwnedResourceRecord>, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let run = &state.runs[run_index(&state, &expected.run_id)?];
        if run.stage != RunStage::Closed || !consumer_run_matches(run, observed)
            || run.result.as_ref().map(|result| (&result.message_id, &result.kind, &result.outcome, &result.source_output))
                != observed.result.as_ref().map(|result| (&result.message_id, &result.kind, &result.outcome, &result.source_output))
            || run.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch)) != observed.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch))
            || !retirement::blockers(&state, run).is_empty() { return Ok(None) }
        if run.close_reason == Some(CloseReason::Superseded) { return Ok(None); }
        if expected.kind == OwnedResourceKind::AttemptDirectory && !attempt_directory_released(&state, run) { return Ok(None); }
        let registered = state.owned_resources.iter().find(|r| r.resource_id == expected.resource_id)
            .ok_or_else(|| error("resource_not_found", "Registered cleanup resource disappeared"))?;
        let unresolved = Self::consumers_unresolved(&state, registered);
        if Self::known_writers_active(&state, registered) { return Ok(None) }
        if unresolved != expected.consumers { return Ok(None) }
        if registered.kind != OwnedResourceKind::FixtureSet
            && registered.cleanup.effects.iter().any(|e| matches!(e.state, CleanupEffectState::Intent | CleanupEffectState::Unknown)) { return Ok(None) }
        if matches!(registered.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch | OwnedResourceKind::Scratch | OwnedResourceKind::AttemptDirectory)
            && (!unresolved.is_empty() || run.attachment.as_ref().is_none_or(|a| !Self::writer_quiescent(&state, run, &a.attachment_id, a.epoch))) { return Ok(None) }
        if matches!(registered.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch)
            && !git_integration_eligible(run) && !merged.is_some_and(|proof| proof.matches_state(&state, run)) { return Ok(None) }
        let Some(resource) = state.owned_resources.iter_mut().find(|r| r.resource_id == expected.resource_id) else { return Ok(None) };
        if resource.identity != expected.identity || resource.cleanup != expected.cleanup || resource.ownership != ResourceOwnership::Owned
            || resource.authority != CleanupAuthority::OwnedWhenSafe { return Ok(None) }
        let at = now();
        resource.cleanup.state = CleanupState::Running;
        resource.cleanup.reason = None;
        resource.cleanup.effects.push(CleanupEffectReceipt { effect_id: id(), kind: resource.kind,
            identity_digest: identity_digest(&resource.identity)?, state: CleanupEffectState::Intent, evidence: None, updated_at: at.clone() });
        resource.cleanup.updated_at = at;
        let mut intent = resource.clone();
        intent.consumers = unresolved;
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(Some(intent))
    }

    fn record_constituent_receipts(&self, expected: &OwnedResourceRecord, receipts: Vec<CleanupEffectReceipt>) -> Result<(), InspectionError> {
        if receipts.is_empty() { return Ok(()) }
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let Some(resource) = state.owned_resources.iter_mut().find(|r| r.resource_id == expected.resource_id) else { return Ok(()) };
        if resource.identity != expected.identity { return Ok(()) }
        let mut changed = false;
        for receipt in receipts {
            if let Some(existing) = resource.cleanup.effects.iter_mut().find(|r| r.effect_id == receipt.effect_id) {
                if existing.identity_digest != receipt.identity_digest || existing.kind != receipt.kind {
                    return Err(error("cleanup_effect_conflict", "Recovered constituent effect identity changed"));
                }
                if *existing != receipt { *existing = receipt; changed = true; }
            } else { resource.cleanup.effects.push(receipt); changed = true; }
        }
        if changed {
            resource.cleanup.updated_at = now();
            let revision = locked.save(&mut state)?;
            self.revision.send_replace(revision);
        }
        Ok(())
    }

    fn finish_cleanup_effect(&self, intent: &OwnedResourceRecord, outcome: CleanupEffectOutcome) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let Some(resource) = state.owned_resources.iter_mut().find(|r| r.resource_id == intent.resource_id) else { return Ok(()) };
        if resource.identity != intent.identity || resource.cleanup.effects.last().map(|e| &e.effect_id) != intent.cleanup.effects.last().map(|e| &e.effect_id) { return Ok(()) }
        let at = now();
        let receipt = resource.cleanup.effects.last_mut().expect("persisted effect intent");
        receipt.state = if outcome.state == CleanupEffectState::Intent { CleanupEffectState::Unknown } else { outcome.state };
        receipt.evidence = outcome.evidence;
        receipt.updated_at = at.clone();
        resource.cleanup.state = match receipt.state {
            CleanupEffectState::Applied | CleanupEffectState::Absent => CleanupState::Removed,
            CleanupEffectState::Refused if resource.kind == OwnedResourceKind::AttemptDirectory
                && outcome.reason.as_ref().is_some_and(|why| why.code == "outputs_kept") => CleanupState::Kept,
            CleanupEffectState::Refused => CleanupState::Waiting,
            CleanupEffectState::Intent | CleanupEffectState::Unknown => CleanupState::Unknown,
        };
        resource.cleanup.reason = outcome.reason;
        resource.cleanup.updated_at = at;
        for effect in outcome.effects {
            if let Some(existing) = resource.cleanup.effects.iter_mut().find(|r| r.effect_id == effect.effect_id) {
                if existing.identity_digest != effect.identity_digest {
                    return Err(error("cleanup_effect_conflict", "Constituent effect identity changed"));
                }
                *existing = effect;
            } else {
                resource.cleanup.effects.push(effect);
            }
        }
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(())
    }

    /// Publish a concrete boundary once per resource/cause. Durable inbox IDs
    /// survive owner restart; acknowledgement does not cause another reminder.
    fn report_cleanup_holds(&self) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let before = state.messages.len();
        let holds: Vec<_> = state.owned_resources.iter().filter_map(|resource| {
            if resource.cleanup.state == CleanupState::Removed
                || !state.runs.iter().any(|run| run.run_id == resource.run_id && run.stage == RunStage::Closed) { return None; }
            let why = resource.cleanup.reason.as_ref()?;
            if !matches!(resource.cleanup.state, CleanupState::Waiting | CleanupState::WaitingIntegration
                | CleanupState::Kept | CleanupState::Unknown | CleanupState::Stalled) { return None; }
            let root = state.runs.iter().find(|root| root.run_id == resource.root_id && root.parent_run_id.is_none())?;
            if root.stage == RunStage::Closed { return None; }
            // Dedup the cause before rendering diagnostics; owner/pane wakes must
            // not copy resource receipts or rebuild already delivered boundaries.
            let message_id = format!("cleanup:{}:{}", resource.resource_id, why.code);
            if state.messages.iter().any(|message| matches!(message.from, ActorRef::Dispatcher)
                && message.message_id == message_id) { return None; }
            let action = cleanup_hold_action(&why.code);
            let text = format!("Cleanup held for attempt {} ({:?}, resource {}): {} [{}]. {}",
                resource.run_id, resource.kind, resource.resource_id, why.message, why.code, action);
            Some((resource.root_id.clone(), message_id, text))
        }).collect();
        for (root_id, message_id, text) in holds {
            messages::append(&mut state, messages::AppendMessage {
                from: ActorRef::Dispatcher, to_run_id: &root_id, message_id: &message_id,
                kind: MessageKind::Observation, text: &text, in_reply_to: None,
                report: None, stale: false, escalated_from: None,
            })?;
        }
        if state.messages.len() != before {
            let revision = locked.save(&mut state)?;
            self.revision.send_replace(revision);
        }
        Ok(())
    }
}

fn git_integration_eligible(run: &Run) -> bool {
    matches!(run.integration, Integration::Integrated { .. })
        || matches!((&run.integration, &run.source_output),
            (Integration::NotApplicable { .. }, Some(SourceOutput::NonCode { .. })))
}

fn cleanup_hold_action(code: &str) -> &'static str {
    match code {
        "integration_pending" => "Record exact target checks through IntegrationRecord, or merge the accepted head into the root's checked-out target; cleanup reevaluates on the resulting event.",
        "terminal_observation_unavailable" => "Restore the exact runtime observation connection; the next connection/pane event reevaluates automatically without resubmitting an uncertain effect.",
        "terminal_primary_checkout_space" => "Leave this explicitly borrowed primary checkout Space and foreign Spaces untouched; normal Git workers should use an isolated worktree. Do not retry this close or delete state to bypass the guard.",
        "containment_unproven" => "Preserve this exact claim until its owned scope and fresh pane inventory prove drain; old uncontained launches are not retrofitted. Do not substitute PID absence or another scope.",
        "scope_dirty_checkout" | "scope_unaccepted_commits" => "Review and integrate the remaining output separately or retain it; neither the process scope nor checkout is stopped/discarded while this work is protected.",
        "scope_resource_admission" => "Finish and release the exact holder's build/resource request with actual build-process exit proof; never remove its queue entry or steal the grant.",
        "terminal_resource_admissions" => "Finish and release the exact old holder's Held/Queued resource request with process-exit proof; replacement pane retirement will reevaluate on that event.",
        "scope_replacement_authority_unproven" => "Reconcile the committed root WorkerReplace operation and old assignment generation; no acceptance Result or new attachment may stand in for this old scope's retirement authority.",
        "checkout_consumer_present" => "The exact old scope is drained; leave foreign panes and writers untouched, and release the sealed claim only after their checkout use is proven clear.",
        "outputs_kept" => "Publish or move these unique outputs to the Library or project; entry-removal events reevaluate empty-only private directory cleanup. Never discard the output tree.",
        "attempt_unaccepted" => "Salvage or retain this old attempt's outputs separately; cancellation, failure or replacement does not authorize deleting them.",
        "consumers_active" | "consumer_drain_unproven" => "Preserve this checkout and claims until independent producer/consumer exit proof is available; root PID or pane absence is not proof.",
        "terminal_retirement_pending" => "Wait for this exact owned harness to become idle; a fresh pane event reevaluates retirement automatically.",
        "terminal_outcome_unknown" | "effect_outcome_unknown" => "Inspect the exact uncertain effect/resource; do not resubmit an unknown close or deletion.",
        "ownership_unproven" | "creation_identity_unavailable" | "creation_provenance_conflict" | "terminal_identity_changed" =>
            "Preserve this borrowed, unknown or changed resource; do not adopt it or substitute another identity.",
        "superseded_work_preserved" => "Review the old attempt separately; never discard its dirty/unmerged output or release its claims because the successor finished.",
        _ => "Inspect the named attempt/resource and resolve this concrete guard; cleanup finish reevaluates existing authority without bypassing it.",
    }
}

/// Shares the existing retirement journal's close intent/outcome in the same
/// store transaction. This creates no second terminal executor or stop proof.
pub(in crate::orchestration) fn sync_retirement_receipts(state: &mut OrchestrationState, run_index: usize) {
    let run = &state.runs[run_index];
    let Some(retirement) = &run.retirement else { return };
    let outcome = match &retirement.state {
        RetirementState::CloseIntent { .. } => CleanupEffectState::Intent,
        RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. } => CleanupEffectState::Applied,
        RetirementState::Retired { .. } => CleanupEffectState::Absent,
        RetirementState::Unknown { phase: RetirementPhase::TerminalClose, .. } => CleanupEffectState::Unknown,
        RetirementState::Retained { reason: RetainReason::PrimaryCheckoutSpace, .. } => CleanupEffectState::Refused,
        _ => return,
    };
    for resource in &mut state.owned_resources {
        if resource.run_id != run.run_id || !matches!(&resource.identity, OwnedResourceIdentity::WorkerTerminal { identity }
            if retirement.identity.as_ref().is_some_and(|retired| retirement::receipt_identity_matches(identity, retired))) { continue; }
        let receipt = CleanupEffectReceipt { effect_id: retirement.retirement_id.clone(),
            kind: OwnedResourceKind::WorkerTerminal, identity_digest: resource.resource_id.clone(),
            state: outcome, evidence: None, updated_at: retirement.updated_at.clone() };
        if let Some(existing) = resource.cleanup.effects.iter_mut().find(|effect| effect.effect_id == receipt.effect_id) {
            *existing = receipt;
        } else { resource.cleanup.effects.push(receipt); }
    }
}

fn terminal_evaluation(resource: &OwnedResourceRecord, run: &Run) -> CleanupEvaluation {
    let OwnedResourceIdentity::WorkerTerminal { identity } = &resource.identity else { unreachable!() };
    let Some(retirement) = &run.retirement else {
        return CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "terminal_retirement_pending", "Terminal retirement requires an accepted idle worker or proven quiescent cancellation"));
    };
    if !retirement.identity.as_ref().is_some_and(|retired| retirement::receipt_identity_matches(identity, retired)) {
        return CleanupEvaluation::Unknown(reason(CleanupReasonClass::Authority, "terminal_identity_changed", "Original terminal retirement identity differs"));
    }
    match &retirement.state {
        RetirementState::Retired { .. } => CleanupEvaluation::Absent,
        RetirementState::Retained { reason: RetainReason::ObservationUnavailable, .. } =>
            CleanupEvaluation::Waiting(reason(CleanupReasonClass::Transient, "terminal_observation_unavailable",
                "Fresh endpoint/pane/process evidence is unavailable; preserved terminal will be reobserved on reconnection or the next physical event")),
        RetirementState::Retained { reason: RetainReason::ForegroundProcess, .. } =>
            CleanupEvaluation::Waiting(reason(CleanupReasonClass::Transient, "terminal_retirement_pending",
                "An extra foreground process protects this pane; physical events and a bounded targeted retirement retry reobserve its exact idle identity")),
        RetirementState::Retained { reason: RetainReason::PrimaryCheckoutSpace, .. } =>
            CleanupEvaluation::Kept(reason(CleanupReasonClass::Authority, "terminal_primary_checkout_space",
                "Automatic closure was refused before pane.close: this worker used an explicitly borrowed primary checkout Space, whose source group cannot atomically exclude foreign Spaces; the terminal and source group were retained")),
        RetirementState::Retained { reason: retained, .. } => CleanupEvaluation::Kept(reason(CleanupReasonClass::UserWork,
            &format!("terminal_retained_{retained:?}"), format!("Exact terminal retirement guard refused closure: {retained:?}; preserve its physical identity and user work"))),
        RetirementState::Unknown { .. } => CleanupEvaluation::Unknown(reason(CleanupReasonClass::Reconcilable, "terminal_outcome_unknown", "Actual terminal retirement remains unconfirmed")),
        RetirementState::Waiting { blockers } if blockers.contains(&RetirementBlocker::ResourceAdmissions) =>
            CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "terminal_resource_admissions",
                "Exact replaced-worker pane remains protected while its Held/Queued resource admission is unresolved")),
        _ => CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "terminal_retirement_pending", "Existing guarded retirement is still pending")),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod merged_tests;
