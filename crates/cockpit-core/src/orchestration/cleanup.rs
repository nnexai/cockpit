use sha2::{Digest, Sha256};
use super::*;
use super::mutate::resume;
use super::store::DurableOperationKind;
use crate::projects::ProjectService;

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
    }
}

fn creation_operation(identity: &OwnedResourceIdentity) -> &str {
    match identity {
        OwnedResourceIdentity::WorkerTerminal { identity } => &identity.launch_tag,
        OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::Space { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::Scratch { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::FixtureSet { identity } => &identity.creation_operation_id,
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

impl OrchestrationService {
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
            OwnedResourceIdentity::Worktree { .. } | OwnedResourceIdentity::Branch { .. } | OwnedResourceIdentity::Space { .. } => {
                if run.setup.as_ref().and_then(|s| s.operation_id.as_deref()) != Some(creation_operation(&request.resource)) {
                    return Err(error("resource_creation_conflict", "Resource was not created by this run setup"));
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

    /// Startup/reconnect capture reads original creation receipts, never adopts
    /// a path merely because it currently exists. The same proof is one record.
    pub async fn capture_owned_resources(&self, projects: &ProjectService) -> Result<(), InspectionError> {
        let saved = self.store.lock()?.read()?;
        for run in saved.runs.iter().filter(|run| run.kind == RunKind::Worker) {
            let setup_operation = run.setup.as_ref().and_then(|s| s.operation_id.as_deref());
            let captured = setup_operation.is_some_and(|operation| saved.owned_resources.iter().any(|resource|
                resource.run_id == run.run_id && resource.creation_operation_id.as_deref() == Some(operation)));
            let (mut identities, limitation) = if captured || setup_operation.is_none() {
                (Vec::new(), false)
            } else {
                let (identities, failed) = match projects.owned_resources_for_run(run).await {
                    Ok(identities) => (identities, false),
                    Err(_) => (Vec::new(), true),
                };
                let limitation = failed || match projects.owned_creation_limitation_for_run(run).await {
                    Ok(limitation) => limitation.is_some(),
                    Err(_) => true,
                };
                (identities, limitation)
            };
            let terminal_captured = run.attachment.as_ref().is_some_and(|attachment| saved.owned_resources.iter().any(|resource|
                matches!(&resource.identity, OwnedResourceIdentity::WorkerTerminal { identity }
                    if resource.run_id == run.run_id && identity.attachment_id.as_deref() == Some(attachment.attachment_id.as_str())
                        && identity.attachment_epoch == Some(attachment.epoch))));
            if !terminal_captured {
                if let Some(identity) = retirement::identity_for(run) {
                    identities.push(OwnedResourceIdentity::WorkerTerminal { identity });
                }
            }
            if identities.is_empty() && !limitation { continue; }
            let locked = self.store.lock()?;
            let mut state = locked.read()?;
            let index = run_index(&state, &run.run_id)?;
            let fresh = &state.runs[index];
            if fresh.attempt != run.attempt || fresh.setup.as_ref().and_then(|s| s.operation_id.as_ref()) != run.setup.as_ref().and_then(|s| s.operation_id.as_ref()) { continue; }
            let before = state.owned_resources.len();
            let mut conflict = false;
            for identity in identities {
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
        let queue = self.store.lock()?.read()?.owned_resources;
        for queued in queue {
            if queued.cleanup.state == CleanupState::Removed { continue; }
            let Some(_lease) = self.execution_lease(&queued.run_id)? else { continue };
            if matches!(queued.kind, OwnedResourceKind::FixtureSet | OwnedResourceKind::Scratch)
                && queued.cleanup.effects.iter().any(|effect| matches!(effect.state, CleanupEffectState::Intent | CleanupEffectState::Unknown)) {
                if let Ok(receipts) = super::fixture_resources::effect_receipts(&queued).await {
                    self.record_constituent_receipts(&queued, receipts)?;
                }
            }
            let (resource, run, blocked, drained) = {
                let state = self.store.lock()?.read()?;
                let Some(resource) = state.owned_resources.iter().find(|r| r.resource_id == queued.resource_id) else { continue };
                let run = &state.runs[run_index(&state, &resource.run_id)?];
                let blocked = retirement::blockers(&state, run);
                let mut observed = resource.clone();
                observed.consumers = Self::consumers_unresolved(&state, resource);
                let writers = Self::known_writers_active(&state, resource);
                let drained = run.attachment.as_ref().is_some_and(|a| Self::writer_quiescent(&state, run, &a.attachment_id, a.epoch));
                (observed, run.clone(), !blocked.is_empty() || writers, drained)
            };
            let evaluation = if resource.ownership == ResourceOwnership::Unknown {
                CleanupEvaluation::Unknown(resource.cleanup.reason.clone().unwrap_or_else(|| reason(CleanupReasonClass::Authority, "ownership_unproven", "Unknown creation authority protects this resource")))
            } else if resource.ownership != ResourceOwnership::Owned || resource.authority != CleanupAuthority::OwnedWhenSafe {
                CleanupEvaluation::Kept(reason(CleanupReasonClass::Authority, "ownership_unproven", "Borrowed resources are retained"))
            } else if resource.kind == OwnedResourceKind::WorkerTerminal {
                terminal_evaluation(&resource, &run)
            } else if run.stage != RunStage::Closed {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "consumers_active", "Run or descendant consumers are still active"))
            } else if matches!(resource.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch)
                && !(matches!(run.integration, Integration::Integrated { .. })
                    || matches!((&run.integration, &run.source_output), (Integration::NotApplicable { .. }, Some(SourceOutput::NonCode { .. })))) {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "integration_pending", "Accepted source output must be integrated before Git removal"))
            } else if blocked {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "consumers_active", "Known descendant or checkout writer consumers remain protected"))
            } else if matches!(resource.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch | OwnedResourceKind::Scratch)
                && (!resource.consumers.is_empty() || !drained) {
                CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "consumer_drain_unproven", "Known consumers have not supplied a proven sealed drain"))
            } else {
                let inspection = match resource.kind {
                    OwnedResourceKind::FixtureSet | OwnedResourceKind::Scratch => super::fixture_resources::inspect_owned_resource(&resource, &run).await,
                    _ => projects.inspect_owned_resource(&resource, &run).await,
                };
                inspection.unwrap_or_else(|_| CleanupEvaluation::Unknown(reason(CleanupReasonClass::Transient, "inspection_unavailable", "Actual resource outcome could not be inspected")))
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
            let intent = self.begin_cleanup_effect(&resource, &run)?;
            let Some(intent) = intent else { continue };
            let outcome = match resource.kind {
                OwnedResourceKind::FixtureSet | OwnedResourceKind::Scratch => super::fixture_resources::apply_owned_resource(&intent, &run).await,
                _ => projects.apply_owned_resource(&intent, &run).await,
            }.unwrap_or_else(|_| CleanupEffectOutcome { state: CleanupEffectState::Unknown, evidence: None,
                reason: Some(reason(CleanupReasonClass::Transient, "effect_outcome_unknown", "Effect did not return a confirmed outcome")), effects: Vec::new() });
            self.finish_cleanup_effect(&intent, outcome)?;
        }
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

    fn begin_cleanup_effect(&self, expected: &OwnedResourceRecord, observed: &Run) -> Result<Option<OwnedResourceRecord>, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let run = &state.runs[run_index(&state, &expected.run_id)?];
        if run.stage != RunStage::Closed || run.attempt != observed.attempt || run.integration != observed.integration
            || run.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch)) != observed.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch))
            || !retirement::blockers(&state, run).is_empty() { return Ok(None) }
        let registered = state.owned_resources.iter().find(|r| r.resource_id == expected.resource_id)
            .ok_or_else(|| error("resource_not_found", "Registered cleanup resource disappeared"))?;
        let unresolved = Self::consumers_unresolved(&state, registered);
        if Self::known_writers_active(&state, registered) { return Ok(None) }
        if unresolved != expected.consumers { return Ok(None) }
        if registered.kind != OwnedResourceKind::FixtureSet
            && registered.cleanup.effects.iter().any(|e| matches!(e.state, CleanupEffectState::Intent | CleanupEffectState::Unknown)) { return Ok(None) }
        if matches!(registered.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch | OwnedResourceKind::Scratch)
            && (!unresolved.is_empty() || run.attachment.as_ref().is_none_or(|a| !Self::writer_quiescent(&state, run, &a.attachment_id, a.epoch))) { return Ok(None) }
        if matches!(registered.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch)
            && !(matches!(run.integration, Integration::Integrated { .. })
                || matches!((&run.integration, &run.source_output), (Integration::NotApplicable { .. }, Some(SourceOutput::NonCode { .. })))) { return Ok(None) }
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
        RetirementState::Retained { .. } => CleanupEvaluation::Kept(reason(CleanupReasonClass::UserWork, "terminal_retained", "Existing retirement guards retained the terminal")),
        RetirementState::Unknown { .. } => CleanupEvaluation::Unknown(reason(CleanupReasonClass::Reconcilable, "terminal_outcome_unknown", "Actual terminal retirement remains unconfirmed")),
        _ => CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "terminal_retirement_pending", "Existing guarded retirement is still pending")),
    }
}

#[cfg(test)]
mod tests;
