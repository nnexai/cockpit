//! Kernel process-scope release; never infer detached consumer exit from a PID.
use super::*;
use crate::process_scope::{self, ScopeState};
use super::super::store::{DurableEffectReceipt, DurableOperationRecord, ScopeReleaseOutcome};
use std::{path::Path, time::Duration};

fn holder(run: &Run) -> Option<AttachmentReference> {
    let attachment = run.attachment.as_ref()?;
    Some(AttachmentReference { run_id: run.run_id.clone(), attachment_id: attachment.attachment_id.clone(),
        attachment_epoch: attachment.epoch })
}

fn resource_admissions(state: &OrchestrationState, run: &Run) -> bool {
    if run.attachment.is_none() { return true; }
    state.resource_requests.iter().any(|request| holder_matches(run, &request.holder)
        && matches!(request.state, ResourceRequestState::Held | ResourceRequestState::Queued))
}

fn holder_matches(run: &Run, holder: &AttachmentReference) -> bool {
    holder.run_id == run.run_id && run.attachment.as_ref().is_some_and(|attachment|
        attachment.attachment_id == holder.attachment_id && attachment.epoch == holder.attachment_epoch)
}

fn retired_scope_holder(state: &OrchestrationState, run: &Run) -> bool {
    run.kind == RunKind::Worker && run.stage == RunStage::Closed
        && run.cleanup_policy == Some(WorkerCleanupPolicy::OwnedWhenSafe)
        && run.retirement.as_ref().is_some_and(|record| matches!(record.state, RetirementState::Retired { .. })
            && retirement::authority_matches(state, run, record))
        && run.attachment.as_ref().is_some_and(|attachment|
            attachment.launch_mode == Some(AttachmentLaunchMode::DirectProcess) && attachment.process_scope.is_some())
        && state.checkout_claims.iter().any(|claim| Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()
            && matches!(claim.state, claims::ClaimState::Held | claims::ClaimState::Reserved | claims::ClaimState::Quiescing)
            && claim.holder.as_ref().is_some_and(|holder| holder_matches(run, holder)))
        && retirement::blockers(state, run).is_empty() && !resource_admissions(state, run)
}

fn stop_eligible(state: &OrchestrationState, run: &Run) -> bool {
    run.close_reason == Some(CloseReason::Accepted) && retired_scope_holder(state, run) && stop_output_proven(run)
}

fn replacement_stop_eligible(state: &OrchestrationState, run: &Run) -> bool {
    run.close_reason == Some(CloseReason::Superseded) && retired_scope_holder(state, run)
        && run.retirement.as_ref().is_some_and(|record| record.trigger == RetirementTrigger::Replacement
            && retirement::replacement_authority_matches(state, run, record))
}

fn other_checkout_consumers_clear(state: &OrchestrationState, run: &Run) -> bool {
    let Some(checkout) = run.checkout.as_ref() else { return false };
    !state.checkout_claims.iter().any(|claim| Some(claim.claim_id.as_str()) != run.checkout_claim_id.as_deref()
        && claim.state != claims::ClaimState::Released && claims::checkouts_conflict(&claim.checkout, checkout))
        && !state.runs.iter().any(|other| other.run_id != run.run_id
            && other.checkout.as_ref().is_some_and(|other_checkout| claims::checkouts_conflict(other_checkout, checkout))
            && other.attachment.as_ref().is_some_and(|attachment|
                !OrchestrationService::writer_quiescent(state, other, &attachment.attachment_id, attachment.epoch)))
}

fn passed_checks(checks: &[CheckOutcome]) -> bool {
    !checks.is_empty() && checks.iter().all(|check| check.outcome == CheckStatus::Passed
        && check.limitation.as_deref().is_none_or(|limitation| limitation.trim().is_empty()))
}

fn stop_output_proven(run: &Run) -> bool {
    let Some(source) = run.source_output.as_ref() else { return false };
    let (result_message_id, task_revision) = match source {
        SourceOutput::Code { result_message_id, task_revision, .. }
        | SourceOutput::NonCode { result_message_id, task_revision, .. } => (result_message_id, task_revision),
    };
    if run.result.as_ref().is_none_or(|result| result.outcome != Some(ReportOutcome::Succeeded)
        || result.message_id != *result_message_id || result.source_output.as_ref() != Some(source)) {
        return false;
    }
    match (&run.integration, source) {
        (Integration::Integrated { source: integrated, accepted_result_message_id, accepted_task_revision,
            checks, verified_at, .. }, SourceOutput::Code { .. }) =>
            !verified_at.is_empty() && integrated == source && accepted_result_message_id == result_message_id
                && accepted_task_revision == task_revision && passed_checks(checks),
        (Integration::NotApplicable { result_message_id: exempt_result, reason },
            SourceOutput::NonCode { artifacts, focused_checks, .. }) =>
            exempt_result == result_message_id && !reason.trim().is_empty()
                && !artifacts.is_empty() && passed_checks(focused_checks),
        _ => false,
    }
}

fn canonical_stop_eligible(locked: &super::super::store::LockedStore<'_>, state: &OrchestrationState,
    run: &Run) -> Result<bool, InspectionError> {
    if run.close_reason == Some(CloseReason::Superseded) { return Ok(replacement_stop_eligible(state, run)); }
    if !stop_eligible(state, run) { return Ok(false); }
    let Some(task_id) = run.task_id.as_deref() else { return Ok(false) };
    if projection::current_task_run(state, &run.root_id, task_id).is_none_or(|current| current.run_id != run.run_id) {
        return Ok(false);
    }
    let revision = match run.source_output.as_ref() {
        Some(SourceOutput::Code { task_revision, .. } | SourceOutput::NonCode { task_revision, .. }) => task_revision,
        None => return Ok(false),
    };
    let document = locked.tasks(&run.root_id)?;
    Ok(document.task(task_id)?.checked && document.checked_matches_revision(task_id, revision)?)
}

async fn stop_output_preflight(run: &Run) -> Result<(), InspectionError> {
    if run.retirement.as_ref().is_some_and(|record| record.trigger == RetirementTrigger::Replacement) {
        // Root WorkerReplace authorizes stopping the idle old scope, not any
        // deletion, clean-tree assertion or change to old completion metadata.
        let checkout = run.checkout.as_ref().ok_or_else(|| error("containment_unproven", "Exact old checkout receipt is unavailable"))?;
        return project_identity::revalidate_checkout(checkout);
    }
    match run.source_output.as_ref() {
        Some(SourceOutput::Code { .. }) => clean_accepted_head(run).await,
        Some(SourceOutput::NonCode { .. }) if stop_output_proven(run) => {
            let checkout = run.checkout.as_ref().ok_or_else(|| error("containment_unproven", "Exact checkout receipt is unavailable"))?;
            // Non-code evidence is the root-reviewed Result and focused-check
            // attestation. Artifact strings are identities, not filesystem hashes.
            // No Git or borrowed-directory deletion authority is inferred here.
            project_identity::revalidate_checkout(checkout)
        }
        _ => Err(error("scope_completion_unproven", "Accepted output lacks exact integration/exemption and passed completion evidence")),
    }
}

async fn clean_accepted_head(run: &Run) -> Result<(), InspectionError> {
    let checkout = run.checkout.as_ref().ok_or_else(|| error("containment_unproven", "Exact checkout receipt is unavailable"))?;
    project_identity::revalidate_checkout(checkout)?;
    let SourceOutput::Code { head_commit, .. } = run.source_output.as_ref()
        .ok_or_else(|| error("integration_pending", "Stopping an owned scope requires accepted integrated code output"))? else {
        return Err(error("integration_pending", "Stopping an owned scope requires verified target integration"));
    };
    let root = Path::new(&checkout.path);
    if crate::space_git::git_line(root, &["rev-parse", "--verify", "HEAD^{commit}"]).await? != *head_commit {
        return Err(error("scope_unaccepted_commits", "Later or unaccepted commits protect this attempt and its scope"));
    }
    let status = crate::space_git::git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"]).await?;
    if !status.status.success() { return Err(error("containment_unproven", "Git cleanliness could not be inspected")); }
    if !status.stdout.is_empty() { return Err(error("scope_dirty_checkout", "Uncommitted or untracked work protects this attempt and its scope")); }
    project_identity::revalidate_checkout(checkout)
}

impl OrchestrationService {
    /// Owner event path: seal the exact holder, optionally stop its owned accepted
    /// scope, then obtain fresh kernel+pane evidence before releasing its claim.
    pub(crate) async fn cleanup_worker_scopes(&self, herdr: &dyn super::super::herdr::OrchestrationHerdr) -> Result<(), InspectionError> {
        let saved = self.store.lock()?.read()?;
        for queued in saved.runs.iter().filter(|run| run.kind == RunKind::Worker && run.stage == RunStage::Closed
            && saved.checkout_claims.iter().any(|claim| Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()
                && matches!(claim.state, claims::ClaimState::Reserved | claims::ClaimState::Held | claims::ClaimState::Quiescing))) {
            let Some(_lease) = self.execution_lease(&queued.run_id)? else { continue };
            let run = self.run_for_review(&queued.session_id, &queued.run_id)?;
            let Some(checkout) = run.checkout.as_ref() else { continue };
            let Some(attachment) = run.attachment.as_ref() else { continue };
            let Some(scope) = attachment.process_scope.as_ref() else {
                self.scope_hold(&run, "containment_unproven", "This launch has no exact owned process-scope receipt; preserve unknown consumers and claims. Existing launches are never retrofitted.")?;
                continue;
            };
            let observation = match process_scope::observe(scope).await {
                Ok(observation) => observation,
                Err(_) => {
                    self.scope_hold(&run, "containment_unproven", "Exact owned scope observation is unavailable; preserve the claim and reobserve on a physical or owner event")?;
                    continue;
                }
            };
            if let ScopeState::Unknown { reason } = &observation.state {
                self.scope_hold(&run, "containment_unproven", reason)?;
                continue;
            }
            let (can_stop, admissions, canonical_problem) = {
                let locked = self.store.lock()?;
                let state = locked.read()?;
                let current = &state.runs[run_index(&state, &run.run_id)?];
                if !scope_run_matches(current, &run) { continue; }
                let admissions = resource_admissions(&state, current);
                match canonical_stop_eligible(&locked, &state, current) {
                    Ok(eligible) => (eligible && !admissions, admissions, None),
                    Err(why) => (false, admissions, Some(why.message)),
                }
            };
            if admissions {
                self.scope_hold(&run, "scope_resource_admission",
                    "A Held or Queued build/resource request belongs to this exact holder; finish and release that request with its actual process-exit proof before scope cleanup. The queue is never discarded.")?;
                continue;
            }
            if matches!(observation.state, ScopeState::Populated) {
                if !can_stop {
                    if run.close_reason == Some(CloseReason::Accepted)
                        && run.retirement.as_ref().is_some_and(|record| matches!(record.state, RetirementState::Retired { .. })) {
                        self.scope_hold(&run, "scope_completion_unproven", canonical_problem.as_deref().unwrap_or(
                            "Exact accepted completion evidence or canonical task revision is not current; review this Result's artifacts and required passed checks before any scope stop"))?;
                    }
                    if run.close_reason == Some(CloseReason::Superseded)
                        && run.retirement.as_ref().is_some_and(|record| matches!(record.state, RetirementState::Retired { .. })) {
                        self.scope_hold(&run, "scope_replacement_authority_unproven",
                            "The committed root WorkerReplace authority or exact old assignment/scope is not current; preserve the old scope and claim until the liaison reconciles that replacement")?;
                    }
                    continue;
                }
                if let Err(why) = stop_output_preflight(&run).await {
                    self.scope_hold(&run, &why.code, &why.message)?;
                    continue;
                }
            }
            let Some(sealed) = self.seal_scope_release(&run)? else { continue };
            let Some(sealed_scope) = sealed.attachment.as_ref().and_then(|attachment| attachment.process_scope.as_ref()) else { continue };
            let mut observed = match process_scope::observe(sealed_scope).await {
                Ok(observed) => observed,
                Err(_) => { self.scope_unknown(&sealed, "Exact scope observation failed after the producer seal")?; continue; }
            };
            if matches!(observed.state, ScopeState::Populated) && can_stop {
                // Foreign checkout panes are not a stop gate: only our exact
                // sealed scope is affected. The source daemon incarnation is.
                let Some(identity) = sealed.retirement.as_ref().and_then(|record| record.identity.as_ref()) else {
                    self.scope_unknown(&sealed, "The retired pane has no immutable source-server identity; no scope stop is authorized")?;
                    continue;
                };
                match herdr.runtime(&sealed.session_id).await {
                    Ok(runtime) if retirement::server_fence(identity, &runtime).is_ok() => {}
                    Ok(_) => {
                        self.scope_unknown(&sealed, "Fresh source-server incarnation is changed or unproven; preserve the exact sealed scope without stopping it")?;
                        continue;
                    }
                    Err(_) => {
                        self.scope_unknown(&sealed, "Fresh source-server incarnation observation is unavailable; no scope stop is authorized")?;
                        continue;
                    }
                }
                // Recheck canonical completion and its code/non-code output gate
                // own adjacent InvocationID/cgroup preflight. No user/root scope.
                let eligible = {
                    let locked = self.store.lock()?;
                    let state = locked.read()?;
                    let current = &state.runs[run_index(&state, &sealed.run_id)?];
                    if scope_run_matches(current, &sealed) { canonical_stop_eligible(&locked, &state, current) }
                    else { Ok(false) }
                };
                match eligible {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(why) => { self.scope_unknown(&sealed, &why.message)?; continue; }
                }
                if let Err(why) = stop_output_preflight(&sealed).await {
                    self.scope_unknown(&sealed, &why.message)?;
                    continue;
                }
                observed = match process_scope::stop_exact(sealed_scope, Duration::from_secs(20)).await {
                    Ok(observed) => observed,
                    Err(_) => { self.scope_unknown(&sealed, "Owned scope stop outcome is unconfirmed; never stop another invocation")?; continue; }
                };
                if matches!(observed.state, ScopeState::Drained) {
                    self.scope_outcome(&sealed, ScopeReleaseOutcome::Stopped)?;
                }
            }
            if !matches!(observed.state, ScopeState::Drained) {
                self.scope_unknown(&sealed, "Owned scope has not proved every contained descendant drained; claim remains sealed")?;
                continue;
            }
            // Re-observation is read-only; a recovered intent never infers drain
            // from its previous stop result or the main process's absence.
            observed = match process_scope::observe(sealed_scope).await {
                Ok(observed) => observed,
                Err(_) => { self.scope_unknown(&sealed, "Fresh drained-scope observation is unavailable")?; continue; }
            };
            let clear = match project_identity::checkout_consumers_clear(herdr, &sealed, checkout).await {
                Ok(clear) => clear,
                Err(_) => { self.scope_unknown(&sealed, "Fresh pane working-directory inventory is unavailable")?; continue; }
            };
            let known_clear = {
                let state = self.store.lock()?.read()?;
                other_checkout_consumers_clear(&state, &sealed)
            };
            if !clear || !known_clear {
                self.scope_outcome(&sealed, ScopeReleaseOutcome::Drained)?;
                self.scope_hold(&sealed, "checkout_consumer_present",
                    "The exact owned scope is drained, but other checkout consumers are present or not proven clear; preserve those panes/processes and the Quiescing claim")?;
                continue;
            }
            let proof = match self.observe_writer_exit(&sealed, checkout, Some(&observed), clear) {
                Ok(proof) => proof,
                Err(why) => { self.scope_unknown(&sealed, &why.message)?; continue; }
            };
            self.release_scope_claim(&sealed, &proof)?;
        }
        Ok(())
    }

    fn seal_scope_release(&self, expected: &Run) -> Result<Option<Run>, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if run.stage != RunStage::Closed || !scope_run_matches(run, expected)
            || resource_admissions(&state, run) { return Ok(None); }
        let Some(scope) = run.attachment.as_ref().and_then(|attachment| attachment.process_scope.clone()) else { return Ok(None) };
        let Some(holder) = holder(run) else { return Ok(None) };
        let Some(claim) = state.checkout_claims.iter().find(|claim|
            Some(claim.claim_id.as_str()) == run.checkout_claim_id.as_deref()).cloned() else { return Ok(None) };
        if claim.state == claims::ClaimState::Released { return Ok(None); }
        let operation_id = scope_operation(run);
        let root_id = run.root_id.clone();
        let revision = claims::seal_for_release(&mut state, &claim.claim_id, &holder, claim.revision, &operation_id)?;
        if !state.operations.iter().any(|record| record.operation_id == operation_id
            && record.kind == DurableOperationKind::ScopeRelease) {
            let at = now();
            state.operations.push(DurableOperationRecord { operation_id: operation_id.clone(), actor_run_id: None,
                principal_key: format!("dispatcher-scope:{}", expected.run_id), root_id,
                kind: DurableOperationKind::ScopeRelease, request_digest: identity_digest_scope(&scope)?,
                run_id: Some(expected.run_id.clone()), resume_operation_id: None, result: None,
                effect_receipts: vec![DurableEffectReceipt::ScopeRelease { unit: scope.unit, invocation_id: scope.invocation_id,
                    claim_id: claim.claim_id, outcome: ScopeReleaseOutcome::Intent }],
                created_at: at.clone(), updated_at: at });
        } else if revision == claim.revision { return Ok(Some(state.runs[index].clone())); }
        let sealed = state.runs[index].clone();
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(Some(sealed))
    }

    fn release_scope_claim(&self, expected: &Run, proof: &claims::WriterExitProof) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        if !scope_run_matches(&state.runs[index], expected) || resource_admissions(&state, &state.runs[index])
            || !other_checkout_consumers_clear(&state, &state.runs[index]) { return Ok(()); }
        let claim_id = state.runs[index].checkout_claim_id.clone().ok_or_else(|| error("checkout_claim_missing", "Sealed holder lost its claim"))?;
        claims::release_with_proof(&mut state, &claim_id, proof)?;
        update_scope_outcome(&mut state, expected, ScopeReleaseOutcome::Released)?;
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(())
    }

    fn scope_outcome(&self, expected: &Run, outcome: ScopeReleaseOutcome) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        if update_scope_outcome(&mut state, expected, outcome)? {
            let revision = locked.save(&mut state)?;
            self.revision.send_replace(revision);
        }
        Ok(())
    }

    fn scope_unknown(&self, expected: &Run, message: &str) -> Result<(), InspectionError> {
        self.scope_outcome(expected, ScopeReleaseOutcome::Unknown)?;
        self.scope_hold(expected, "containment_unproven", message)
    }

    fn scope_hold(&self, expected: &Run, code: &str, message: &str) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let run = &state.runs[run_index(&state, &expected.run_id)?];
        if !scope_run_matches(run, expected) || run.root_id != expected.root_id { return Ok(()); }
        let why = reason(if code.starts_with("scope_") { CleanupReasonClass::UserWork } else { CleanupReasonClass::Authority }, code, message);
        let mut changed = false;
        let mut has_resource_target = false;
        for resource in state.owned_resources.iter_mut().filter(|resource| resource.run_id == expected.run_id
            && resource.cleanup.state != CleanupState::Removed && resource.ownership == ResourceOwnership::Owned
            && resource.authority == CleanupAuthority::OwnedWhenSafe
            && matches!(resource.kind, OwnedResourceKind::Worktree | OwnedResourceKind::Branch | OwnedResourceKind::Scratch | OwnedResourceKind::Space | OwnedResourceKind::AttemptDirectory)) {
            has_resource_target = true;
            if resource.cleanup.state != CleanupState::Kept || resource.cleanup.reason.as_ref() != Some(&why) {
                resource.cleanup.state = CleanupState::Kept;
                resource.cleanup.reason = Some(why.clone());
                resource.cleanup.updated_at = now();
                changed = true;
            }
        }
        // Borrowed checkout attempts own no filesystem row. Their sealed
        // claim can still be genuinely held by a foreign checkout consumer.
        // Report that scope boundary without inventing filesystem ownership.
        if !has_resource_target && state.checkout_claims.iter().any(|claim|
            Some(claim.claim_id.as_str()) == expected.checkout_claim_id.as_deref()
                && matches!(claim.state, claims::ClaimState::Reserved | claims::ClaimState::Held | claims::ClaimState::Quiescing)
                && expected.checkout.as_ref() == Some(&claim.checkout)
                && claim.holder.as_ref().is_some_and(|holder| holder_matches(expected, holder)))
            && state.runs.iter().any(|root| root.run_id == expected.root_id
                && root.parent_run_id.is_none() && root.stage != RunStage::Closed) {
            let epoch = expected.attachment.as_ref().map_or(0, |attachment| attachment.epoch);
            let message_id = format!("scope-cleanup:{}:{epoch}:{code}", expected.run_id);
            if !state.messages.iter().any(|saved| matches!(saved.from, ActorRef::Dispatcher) && saved.message_id == message_id) {
                let text = format!("Cleanup held for attempt {} (checkout claim {}, attachment epoch {epoch}): {message} [{code}]. {}",
                    expected.run_id, expected.checkout_claim_id.as_deref().unwrap_or("unproven"), cleanup_hold_action(code));
                messages::append(&mut state, messages::AppendMessage { from: ActorRef::Dispatcher,
                    to_run_id: &expected.root_id, message_id: &message_id, kind: MessageKind::Observation,
                    text: &text, in_reply_to: None, report: None, stale: false, escalated_from: None })?;
                changed = true;
            }
        }
        if changed { let revision = locked.save(&mut state)?; self.revision.send_replace(revision); }
        Ok(())
    }
}

fn scope_run_matches(current: &Run, expected: &Run) -> bool {
    current.stage == RunStage::Closed && current.close_reason == expected.close_reason
        && current.checkout == expected.checkout && current.checkout_claim_id == expected.checkout_claim_id
        && current.integration == expected.integration && current.cleanup_policy == expected.cleanup_policy
        && current.source_output == expected.source_output && current.assignment_generation == expected.assignment_generation
        && current.result.as_ref().map(|result| (&result.message_id, &result.source_output, result.outcome))
            == expected.result.as_ref().map(|result| (&result.message_id, &result.source_output, result.outcome))
        && current.attachment.as_ref().zip(expected.attachment.as_ref()).is_some_and(|(current, expected)|
            current.attachment_id == expected.attachment_id && current.epoch == expected.epoch
                && current.process_scope == expected.process_scope)
}

fn scope_operation(run: &Run) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID,
        format!("scope-release:{}:{}", run.run_id, run.attachment.as_ref().map_or(0, |attachment| attachment.epoch)).as_bytes()).to_string()
}

fn identity_digest_scope(scope: &OwnedProcessScope) -> Result<String, InspectionError> {
    let bytes = serde_json::to_vec(scope).map_err(|error| super::super::error("scope_identity", &error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn update_scope_outcome(state: &mut OrchestrationState, expected: &Run, outcome: ScopeReleaseOutcome) -> Result<bool, InspectionError> {
    let index = run_index(state, &expected.run_id)?;
    if !scope_run_matches(&state.runs[index], expected) { return Ok(false); }
    let Some(scope) = expected.attachment.as_ref().and_then(|attachment| attachment.process_scope.as_ref()) else { return Ok(false) };
    let operation_id = scope_operation(expected);
    let Some(record) = state.operations.iter_mut().find(|record| record.operation_id == operation_id
        && record.kind == DurableOperationKind::ScopeRelease) else { return Ok(false) };
    let Some(DurableEffectReceipt::ScopeRelease { unit, invocation_id, claim_id, outcome: saved }) = record.effect_receipts.first_mut() else { return Ok(false) };
    if unit != &scope.unit || invocation_id != &scope.invocation_id || Some(claim_id.as_str()) != expected.checkout_claim_id.as_deref() {
        return Err(error("scope_identity_changed", "Release journal differs from its exact scope/claim receipt"));
    }
    if *saved == outcome { return Ok(false); }
    *saved = outcome;
    record.updated_at = now();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture { root: std::path::PathBuf, service: OrchestrationService, run: Run }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("cockpit-scope-cleanup-{}", id()));
            std::fs::create_dir_all(&root).unwrap();
            let (_, checkout) = project_identity::resolve_owned_directory(&root).unwrap();
            let (revision, _) = watch::channel(0);
            let service = OrchestrationService { store: OrchestrationStore::open(&root).unwrap(), revision,
                notifications: std::sync::Mutex::new(None), launch_config: Default::default() };
            let mut run = new_run(&id(), "scope-tests", RunKind::Worker, "generic worker".into(),
                "root", Some("root".into()), None, 1);
            run.stage = RunStage::Closed;
            run.close_reason = Some(CloseReason::Accepted);
            run.cleanup_policy = Some(WorkerCleanupPolicy::OwnedWhenSafe);
            run.launch_config.harness_kind = "generic-test-harness".into();
            let attachment = run.attachment.as_mut().unwrap();
            attachment.launch_mode = Some(AttachmentLaunchMode::DirectProcess);
            attachment.state = AttachmentState::Connected;
            attachment.launch_attempt = 1;
            attachment.machine_id = Some("machine".into());
            attachment.process = Some(NativeProcessIdentity { pid: 42, start_ticks: 7, kernel_boot_id: Some("boot".into()) });
            attachment.process_scope = Some(OwnedProcessScope { kind: ScopeKind::SystemdUserScope,
                unit: "cockpit-w-fixture.scope".into(), invocation_id: "a".repeat(32),
                control_group: "/fixture/cockpit-w-fixture.scope".into(), cgroup_device: "1".into(),
                cgroup_inode: "2".into(), machine_id: "machine".into(), kernel_boot_id: "boot".into(),
                member: attachment.process.clone().unwrap(), captured_at: now() });
            attachment.location = Some(RunLocation { endpoint_identity: "endpoint".into(), session_id: run.session_id.clone(),
                workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(),
                terminal_id: Some("terminal".into()), launch_tag: "scope-test".into(), boot_id: Some("legacy-test-boot".into()), peer_process: None, native_session_id: None });
            let mut launch = dispatch(DispatchStep::Launched);
            launch.launch_attempt = 1;
            launch.launch_tag = Some("scope-test".into());
            launch.endpoint_identity = Some("endpoint".into());
            run.dispatch = Some(launch);
            let checks = vec![CheckOutcome { contract: "contract".into(), command_or_scenario: "target scenario".into(),
                outcome: CheckStatus::Passed, evidence: Some("target evidence".into()), limitation: None }];
            let source = SourceOutput::Code { repository_key: "repository".into(), checkout_key: checkout.key.clone(),
                base_commit: "0".repeat(40), head_commit: "a".repeat(40), commits: vec!["a".repeat(40)],
                task_revision: "scope".into(), result_message_id: "result".into(), focused_checks: checks.clone() };
            run.source_output = Some(source.clone());
            run.result = Some(Report { message_id: "result".into(), kind: ReportKind::Result,
                outcome: Some(ReportOutcome::Succeeded), summary: "complete".into(), plan: None,
                source_output: Some(source.clone()), at: now() });
            run.integration = Integration::Integrated { operation_id: id(), accepted_result_message_id: "result".into(),
                accepted_task_revision: "scope".into(), source, target_repository_key: "repository".into(),
                target_ref: "refs/heads/main".into(), target_commit: "b".repeat(40), method: IntegrationMethod::Ancestor,
                mappings: Vec::new(), checks, integrator_run_id: "root".into(), verified_at: now() };
            run.retirement = retirement::retirement_for_accepted(&run, RetirementTrigger::Accept, "scope", &now());
            run.retirement.as_mut().unwrap().state = RetirementState::Retired { at: now(), terminal: TerminalOutcome::ClosedByCockpit };
            let locked = service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(run);
            let holder = holder(&state.runs[0]).unwrap();
            let claim = claims::reserve_claim(&mut state, &checkout, &holder, &id()).unwrap();
            claims::hold_claim(&mut state, &claim, &holder, 1).unwrap();
            let run = state.runs[0].clone();
            locked.save(&mut state).unwrap();
            drop(locked);
            Self { root, service, run }
        }
    }

    #[test]
    fn accepted_scope_stop_is_harness_neutral_and_never_root_or_superseded() {
        let fixture = Fixture::new();
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert!(stop_eligible(&state, &fixture.run));
        for variant in 0..7 {
            let mut run = fixture.run.clone();
            match variant {
                0 => run.kind = RunKind::Supervisor,
                1 => run.close_reason = Some(CloseReason::Superseded),
                2 => run.close_reason = Some(CloseReason::Cancelled),
                3 => run.integration = Integration::Pending,
                4 => run.retirement.as_mut().unwrap().state = RetirementState::Waiting { blockers: Vec::new() },
                5 => run.attachment.as_mut().unwrap().process_scope = None,
                _ => run.cleanup_policy = None,
            }
            assert!(!stop_eligible(&state, &run), "unsafe stop variant {variant}");
        }
        for status in [CheckStatus::Failed, CheckStatus::NotRun] {
            let mut run = fixture.run.clone();
            if let Integration::Integrated { checks, .. } = &mut run.integration { checks[0].outcome = status; }
            assert!(!stop_eligible(&state, &run));
        }
    }

    #[test]
    fn scope_release_seal_is_durable_idempotent_and_recovered_unknown_does_not_self_wake() {
        let fixture = Fixture::new();
        let sealed = fixture.service.seal_scope_release(&fixture.run).unwrap().unwrap();
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(state.checkout_claims[0].state, claims::ClaimState::Quiescing);
        assert_eq!(sealed.attachment.as_ref().unwrap().state, AttachmentState::Quiescing);
        assert!(matches!(state.operations[0].effect_receipts[0],
            DurableEffectReceipt::ScopeRelease { outcome: ScopeReleaseOutcome::Intent, .. }));
        fixture.service.scope_outcome(&sealed, ScopeReleaseOutcome::Unknown).unwrap();
        let before = std::fs::read(fixture.service.base().join("state.json")).unwrap();
        assert!(fixture.service.seal_scope_release(&sealed).unwrap().is_some());
        fixture.service.scope_outcome(&sealed, ScopeReleaseOutcome::Unknown).unwrap();
        assert_eq!(std::fs::read(fixture.service.base().join("state.json")).unwrap(), before);
        let mut foreign = sealed;
        foreign.attachment.as_mut().unwrap().process_scope.as_mut().unwrap().invocation_id = "f".repeat(32);
        assert!(fixture.service.seal_scope_release(&foreign).unwrap().is_none());
        assert_eq!(std::fs::read(fixture.service.base().join("state.json")).unwrap(), before);
    }

    #[test]
    fn held_or_queued_build_admissions_prevent_stop_and_seal_without_rewriting_queue() {
        for admission in [ResourceRequestState::Held, ResourceRequestState::Queued] {
            let fixture = Fixture::new();
            {
                let locked = fixture.service.store.lock().unwrap();
                let mut state = locked.read().unwrap();
                state.resource_requests.push(ResourceRequestRecord { request_id: id(), sequence: 1,
                    resource_key: "shared-build".into(), source_identity: ResourceSourceIdentity {
                        checkout_key: fixture.run.checkout.as_ref().unwrap().key.clone(), source_commit: "a".repeat(40),
                        source_tree: "b".repeat(40), input_configuration: "input".into(), toolchain_fingerprint: "toolchain".into(),
                    }, holder: holder(&fixture.run).unwrap(), state: admission, build_processes: Vec::new(),
                    machine_id: "machine".into(), blocked_reason: None, updated_at: now() });
                locked.save(&mut state).unwrap();
            }
            let before = std::fs::read(fixture.service.base().join("state.json")).unwrap();
            let state = fixture.service.store.lock().unwrap().read().unwrap();
            assert!(!stop_eligible(&state, &fixture.run));
            assert!(fixture.service.seal_scope_release(&fixture.run).unwrap().is_none());
            assert_eq!(std::fs::read(fixture.service.base().join("state.json")).unwrap(), before);
            assert_eq!(state.resource_requests[0].state, admission);
            assert_eq!(state.checkout_claims[0].state, claims::ClaimState::Held);
        }
    }

    #[tokio::test]
    async fn scope_stop_git_gate_preserves_dirty_and_later_unaccepted_commits() {
        use crate::space_git::tests::git;
        let fixture = Fixture::new();
        let checkout = fixture.root.join("checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        git(&checkout, &["init", "-b", "main"]);
        git(&checkout, &["config", "user.email", "cleanup@example.test"]);
        git(&checkout, &["config", "user.name", "cleanup test"]);
        std::fs::write(checkout.join("contract"), "accepted\n").unwrap();
        git(&checkout, &["add", "."]);
        git(&checkout, &["commit", "-m", "accepted source"]);
        let head = git(&checkout, &["rev-parse", "HEAD"]).trim().to_owned();
        let mut run = fixture.run.clone();
        run.checkout = Some(project_identity::resolve_directory(&checkout, None, None).await.unwrap().1);
        if let Some(SourceOutput::Code { head_commit, .. }) = &mut run.source_output { *head_commit = head; }
        assert!(clean_accepted_head(&run).await.is_ok());
        std::fs::write(checkout.join("draft"), "preserve draft\n").unwrap();
        assert_eq!(clean_accepted_head(&run).await.unwrap_err().code, "scope_dirty_checkout");
        git(&checkout, &["add", "draft"]);
        git(&checkout, &["commit", "-m", "later output not accepted"]);
        assert_eq!(clean_accepted_head(&run).await.unwrap_err().code, "scope_unaccepted_commits");
        assert_eq!(std::fs::read_to_string(checkout.join("draft")).unwrap(), "preserve draft\n");
    }

    #[tokio::test]
    async fn accepted_noncode_scope_stop_requires_exact_exemption_artifacts_and_passed_attestation_without_git() {
        let fixture = Fixture::new();
        std::fs::write(fixture.root.join("artifact.txt"), "actual fixture artifact\n").unwrap();
        let mut run = fixture.run.clone();
        let source = SourceOutput::NonCode { result_message_id: "result".into(), task_revision: "scope".into(),
            artifacts: vec!["artifact.txt".into()], focused_checks: vec![CheckOutcome {
                contract: "non-code output".into(), command_or_scenario: "read fixture artifact".into(),
                outcome: CheckStatus::Passed, evidence: None, limitation: None,
            }] };
        run.source_output = Some(source.clone());
        run.result.as_mut().unwrap().source_output = Some(source);
        run.integration = Integration::NotApplicable { result_message_id: "result".into(),
            reason: "Root reviewed the completed non-code artifact".into() };
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert!(stop_eligible(&state, &run));
        assert!(stop_output_preflight(&run).await.is_ok());
        assert!(!fixture.root.join(".git").exists(), "non-code evidence cannot invent Git");
        assert_eq!(std::fs::read_to_string(fixture.root.join("artifact.txt")).unwrap(), "actual fixture artifact\n");
        for missing in 0..6 {
            let mut changed = run.clone();
            match missing {
                0 => if let Some(SourceOutput::NonCode { artifacts, .. }) = &mut changed.source_output { artifacts.clear(); },
                1 => if let Some(SourceOutput::NonCode { focused_checks, .. }) = &mut changed.source_output { focused_checks.clear(); },
                2 | 3 => if let Some(SourceOutput::NonCode { focused_checks, .. }) = &mut changed.source_output {
                    focused_checks[0].outcome = if missing == 2 { CheckStatus::Failed } else { CheckStatus::NotRun };
                },
                4 => changed.integration = Integration::NotApplicable { result_message_id: "other-result".into(), reason: "wrong Result".into() },
                _ => changed.result.as_mut().unwrap().message_id = "other-result".into(),
            }
            if missing < 4 { changed.result.as_mut().unwrap().source_output = changed.source_output.clone(); }
            assert!(!stop_eligible(&state, &changed), "missing non-code proof variant {missing}");
            assert!(stop_output_preflight(&changed).await.is_err());
        }
        let mut stale = run.clone();
        stale.assignment_generation = Some(999);
        assert!(!scope_run_matches(&stale, &run));
        stale = run.clone();
        stale.result.as_mut().unwrap().message_id = "other-result".into();
        assert!(!scope_run_matches(&stale, &run));
    }

    #[tokio::test]
    async fn replacement_scope_stop_keeps_dirty_output_and_foreign_claim_blocks_only_release() {
        let fixture = Fixture::new();
        std::fs::write(fixture.root.join("old-dirty-output"), "old bytes stay\n").unwrap();
        let mut run = fixture.run.clone();
        run.close_reason = Some(CloseReason::Superseded);
        run.task_id = Some(id());
        run.assignment_generation = Some(1);
        run.result = None;
        run.source_output = None;
        run.integration = Integration::Pending;
        let operation_id = id();
        let successor_id = id();
        run.retirement = retirement::retirement_for_replacement(&run, ReplacementRetirementAuthority {
            operation_id: operation_id.clone(), previous_run_id: run.run_id.clone(), previous_generation: 1,
            replacement_run_id: successor_id.clone(),
        }, &now());
        run.retirement.as_mut().unwrap().state = RetirementState::Retired { at: now(), terminal: TerminalOutcome::ClosedByCockpit };
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.runs[0] = run.clone();
        let mut successor = new_run(&successor_id, &run.session_id, RunKind::Worker, "successor".into(),
            &run.root_id, Some(run.root_id.clone()), run.task_id.clone(), 2);
        successor.assignment_generation = Some(2);
        state.runs.push(successor);
        state.task_ownership.push(TaskOwnership { root_id: run.root_id.clone(), task_id: run.task_id.clone().unwrap(),
            generation: 2, owner: None, schedule: None });
        state.operations.push(DurableOperationRecord { operation_id: operation_id.clone(), actor_run_id: Some(run.root_id.clone()),
            principal_key: "root".into(), root_id: run.root_id.clone(), kind: DurableOperationKind::WorkerReplace,
            request_digest: "committed root replacement".into(), run_id: Some(successor_id.clone()),
            resume_operation_id: None, result: Some(OrchestrationActionResult::WorkerReplaced {
                start: WorkerStartResult { operation_id, run_id: successor_id, attempt: 2, duplicate: false },
                previous_run_id: run.run_id.clone(), assignment_generation: 2,
                placement: ReplacementPlacement::IsolatedDirectory { directory: "/isolated/successor".into() },
            }), effect_receipts: Vec::new(), created_at: now(), updated_at: now() });
        let mut foreign = state.checkout_claims[0].clone();
        foreign.claim_id = id();
        foreign.holder = None; // An unclassified competing writer remains protected.
        state.checkout_claims.push(foreign);
        assert!(replacement_stop_eligible(&state, &run), "a foreign checkout claim does not authorize stopping it or prevent stopping only our old scope");
        assert!(!other_checkout_consumers_clear(&state, &run));
        locked.save(&mut state).unwrap();
        drop(locked);
        assert!(stop_output_preflight(&run).await.is_ok(), "replacement does not require or manufacture clean/integrated old output");
        assert_eq!(std::fs::read_to_string(fixture.root.join("old-dirty-output")).unwrap(), "old bytes stay\n");
        let sealed = fixture.service.seal_scope_release(&run).unwrap().unwrap();
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(state.checkout_claims[0].state, claims::ClaimState::Quiescing);
        assert_eq!(state.checkout_claims[1].state, claims::ClaimState::Held);
        assert!(!stop_eligible(&state, &sealed), "accepted cleanup authority was not broadened");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn released_consumer_authority_requires_exact_private_release_cas_and_journal() {
        use crate::space_git::tests::git;
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new();
        let root = fixture.root.join("git-checkout");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.email", "cleanup@example.test"]);
        git(&root, &["config", "user.name", "cleanup test"]);
        std::fs::write(root.join("artifact"), "accepted output\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "accepted output"]);
        let head = git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
        let (project, checkout) = project_identity::resolve_directory(&root, None, None).await.unwrap();
        let common = checkout.git.as_ref().unwrap().common_dir.clone();
        let directory = std::fs::metadata(&root).unwrap();
        let git_metadata = std::fs::metadata(&common).unwrap();
        let creator = id();
        let identity = OwnedGitIdentity { creation_operation_id: creator.clone(), machine_id: "machine".into(),
            repository_key: common.clone(), repository_root: checkout.path.clone(),
            repository_device: directory.dev().to_string(), repository_inode: directory.ino().to_string(),
            common_dir: common, common_device: git_metadata.dev().to_string(), common_inode: git_metadata.ino().to_string(),
            checkout_path: checkout.path.clone(), checkout_device: checkout.directory_identity.device.clone(),
            checkout_inode: checkout.directory_identity.inode.clone(), checkout_key: checkout.key.clone(),
            branch_ref: Some("refs/heads/main".into()), creation_tip: head.clone() };
        let mut run = fixture.run.clone();
        run.project = Some(project.clone());
        run.checkout = Some(checkout.clone());
        run.setup = Some(SetupSummary { operation_id: Some(creator), generation: Some(1), workspace_id: Some("space".into()),
            checkout_path: checkout.path.clone(), repository_id: None, branch: Some("main".into()), base: Some(head.clone()),
            base_commit: Some(head.clone()), ownership: Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree),
            effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None });
        let source = run.source_output.as_mut().unwrap();
        if let SourceOutput::Code { repository_key, checkout_key, base_commit, head_commit, commits, .. } = source {
            *repository_key = project.key.clone(); *checkout_key = checkout.key.clone();
            *base_commit = head.clone(); *head_commit = head.clone(); *commits = Vec::new();
        }
        run.result.as_mut().unwrap().source_output = Some(source.clone());
        if let Integration::Integrated { source: integrated, target_repository_key, target_commit, .. } = &mut run.integration {
            *integrated = source.clone(); *target_repository_key = project.key; *target_commit = head;
        }
        run.retirement = retirement::retirement_for_accepted(&run, RetirementTrigger::Accept, "scope", &now());
        run.retirement.as_mut().unwrap().state = RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent };
        let mut resource = {
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs[0] = run.clone();
            state.checkout_claims[0].checkout = checkout.clone();
            let index = insert_resource(&mut state.owned_resources, &run, OwnedResourceIdentity::Worktree { identity }).unwrap();
            let resource = state.owned_resources[index].clone();
            locked.save(&mut state).unwrap();
            resource
        };
        resource.consumers.clear();
        assert_eq!(crate::project_teardown::owned_consumers_quiescent(&resource, &run, None).await.unwrap_err().code,
            "consumer_proof_missing", "an empty consumer list without service authority remains fail-closed");
        assert!(fixture.service.owned_consumer_proof(&resource, &run).unwrap().is_none(),
            "empty refs and a retired pane do not imply consumer exit");
        let sealed = fixture.service.seal_scope_release(&run).unwrap().unwrap();
        assert!(fixture.service.owned_consumer_proof(&resource, &sealed).unwrap().is_none(),
            "a sealed intent is not completed drain/release");
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        let attachment = sealed.attachment.as_ref().unwrap();
        let scope = attachment.process_scope.as_ref().unwrap();
        // Trusted observer boundary fixture only. Physical validation below
        // still refuses this mock scope; no adapter effect is invoked.
        let exit = claims::WriterExitProof { incarnation: holder(&sealed).unwrap(),
            claim_revision: state.checkout_claims[0].revision, checkout: Some(checkout),
            machine_id: scope.machine_id.clone(), previous_kernel_boot_id: scope.kernel_boot_id.clone(),
            observed_kernel_boot_id: scope.kernel_boot_id.clone(), all_consumers_absent: true,
            basis: claims::ExitBasis::ScopeDrained { unit: scope.unit.clone(), invocation_id: scope.invocation_id.clone() } };
        fixture.service.release_scope_claim(&sealed, &exit).unwrap();
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        let released = state.runs[0].clone();
        let proof = fixture.service.owned_consumer_proof(&resource, &released).unwrap().unwrap();
        fixture.service.validate_consumer_release(&resource, &released, proof.claim_revision).unwrap();
        assert!(proof.validate(&resource, &released).await.is_err(),
            "cached release authority never substitutes for fresh real scope observation");
        for changed in 0..9 {
            let mut bad = state.clone();
            match changed {
                0 => bad.checkout_claims[0].state = claims::ClaimState::Held,
                1 => bad.checkout_claims[0].transition_operation_id = None,
                2 => bad.runs[0].attachment.as_mut().unwrap().epoch += 1,
                3 => bad.owned_resources[0].ownership = ResourceOwnership::Borrowed,
                4 => bad.runs[0].setup.as_mut().unwrap().operation_id = Some(id()),
                5 => bad.checkout_claims[0].checkout.path = "/different-checkout".into(),
                6 => {
                    let operation = bad.operations.iter_mut().find(|operation| operation.kind == DurableOperationKind::ScopeRelease).unwrap();
                    let DurableEffectReceipt::ScopeRelease { outcome, .. } = &mut operation.effect_receipts[0] else { unreachable!() };
                    *outcome = ScopeReleaseOutcome::Drained;
                }
                7 => {
                    let operation = bad.operations.iter_mut().find(|operation| operation.kind == DurableOperationKind::ScopeRelease).unwrap();
                    let DurableEffectReceipt::ScopeRelease { invocation_id, .. } = &mut operation.effect_receipts[0] else { unreachable!() };
                    *invocation_id = "b".repeat(32);
                }
                _ => {
                    let mut foreign = bad.checkout_claims[0].clone();
                    foreign.claim_id = id(); foreign.state = claims::ClaimState::Held;
                    bad.checkout_claims.push(foreign);
                }
            }
            assert!(released_consumer_revision(&bad, &resource, &released).is_none(), "changed proof guard {changed}");
        }
    }

    #[test]
    fn borrowed_scope_hold_reaches_liaison_once_across_ack_restart_and_requires_pending_claim() {
        let fixture = Fixture::new();
        {
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            let mut root = new_run(&fixture.run.root_id, &fixture.run.session_id, RunKind::Supervisor,
                "liaison".into(), &fixture.run.root_id, None, None, 1);
            root.stage = RunStage::Active;
            state.runs.push(root);
            assert!(state.owned_resources.is_empty(), "borrowed checkout has no fabricated filesystem ownership");
            locked.save(&mut state).unwrap();
        }
        fixture.service.scope_hold(&fixture.run, "checkout_consumer_present", "Exact scope drained; another checkout consumer remains").unwrap();
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(state.messages.len(), 1);
        let notice = &state.messages[0];
        assert!(matches!(notice.from, ActorRef::Dispatcher));
        assert_eq!(notice.kind, MessageKind::Observation);
        assert_eq!(notice.to_run_id, fixture.run.root_id);
        assert!(notice.report.is_none(), "meaningful boundary is not a quiet Progress report");
        assert!(notice.text.contains(&format!("Cleanup held for attempt {}", fixture.run.run_id)));
        assert!(notice.text.contains(fixture.run.checkout_claim_id.as_deref().unwrap()));
        assert!(notice.text.contains("attachment epoch 1"));
        assert!(notice.text.contains("[checkout_consumer_present]"));
        assert!(state.owned_resources.is_empty());
        {
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.messages[0].stage = DeliveryStage::Acked;
            locked.save(&mut state).unwrap();
        }
        let (revision, _) = watch::channel(0);
        let reopened = OrchestrationService { store: OrchestrationStore::open(&fixture.root).unwrap(),
            revision, notifications: std::sync::Mutex::new(None), launch_config: Default::default() };
        let before = reopened.store.lock().unwrap().read().unwrap().revision;
        reopened.scope_hold(&fixture.run, "checkout_consumer_present", "Exact scope drained; another checkout consumer remains").unwrap();
        let state = reopened.store.lock().unwrap().read().unwrap();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.revision, before, "wake, ACK and restart cannot repeat the same held cause");
        reopened.scope_hold(&fixture.run, "scope_observation_unavailable", "Restore the exact scope observer").unwrap();
        assert_eq!(reopened.store.lock().unwrap().read().unwrap().messages.len(), 2, "a different actionable cause gets one new boundary");
        let mut stale = fixture.run.clone();
        stale.attachment.as_mut().unwrap().epoch += 1;
        reopened.scope_hold(&stale, "scope_stale_epoch", "Must not notify for another physical attempt").unwrap();
        assert_eq!(reopened.store.lock().unwrap().read().unwrap().messages.len(), 2);
        {
            let locked = reopened.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.checkout_claims[0].state = claims::ClaimState::Released;
            state.checkout_claims[0].holder = None;
            locked.save(&mut state).unwrap();
        }
        reopened.scope_hold(&fixture.run, "scope_nothing_pending", "Must not invent remaining work").unwrap();
        let state = reopened.store.lock().unwrap().read().unwrap();
        assert_eq!(state.messages.len(), 2);
        assert!(state.owned_resources.is_empty());
    }
}
