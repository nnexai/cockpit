use std::{collections::BTreeSet, path::Path};

use super::{Applied, MutationCtx, resume};
use super::super::*;
use crate::space_git;
use super::super::store::DurableOperationKind;

/// Core-created observations, bound to one semantic request and its current
/// checkout holder. Neither wire fields nor the host can manufacture this proof.
#[derive(Debug, Clone)]
pub struct OutputFacts {
    request_digest: String,
    run_id: String,
    source_checkout: CheckoutIdentity,
    source_claim_id: String,
    source_claim_revision: u64,
    source_holder: AttachmentReference,
    target_checkout: Option<CheckoutIdentity>,
    integration: Option<Integration>,
}

pub(crate) fn output_identity(output: &SourceOutput) -> (&str, &str) {
    match output {
        SourceOutput::Code { result_message_id, task_revision, .. }
        | SourceOutput::NonCode { result_message_id, task_revision, .. } =>
            (result_message_id, task_revision),
    }
}

fn checks(output: &SourceOutput) -> &[CheckOutcome] {
    match output {
        SourceOutput::Code { focused_checks, .. } | SourceOutput::NonCode { focused_checks, .. } => focused_checks,
    }
}

fn validate_checks(checks: &[CheckOutcome]) -> Result<(), InspectionError> {
    if checks.len() > 128 { return Err(error("invalid_source_output", "Too many focused checks")); }
    let mut contracts = BTreeSet::new();
    for check in checks {
        bounded(&check.contract, 1024)?;
        bounded(&check.command_or_scenario, 4096)?;
        if check.contract.trim().is_empty() || check.command_or_scenario.trim().is_empty()
            || !contracts.insert(check.contract.as_str()) {
            return Err(error("invalid_source_output", "Each focused check needs a unique contract and actual command or scenario"));
        }
        if let Some(evidence) = &check.evidence { bounded(evidence, 16384)?; }
        if let Some(limitation) = &check.limitation { bounded(limitation, 4096)?; }
    }
    Ok(())
}

pub(crate) fn require_source_identity(run: &Run, message_id: &str, revision: &str) -> Result<(), InspectionError> {
    let output = run.source_output.as_ref().ok_or_else(|| error("source_output_required", "Successful Result requires its actual code or non-code source output"))?;
    if output_identity(output) != (message_id, revision)
        || run.result.as_ref().and_then(|r| r.source_output.as_ref()) != Some(output) {
        return Err(error("source_output_conflict", "Source output does not belong to this exact Result and task revision"));
    }
    Ok(())
}

fn validate_output_task_revision(output: &SourceOutput, actual_revision: &str) -> Result<(), InspectionError> {
    if output_identity(output).1 != actual_revision {
        return Err(error("source_output_conflict", "Result source output must match the current canonical task revision"));
    }
    Ok(())
}

/// Report owner invokes this before appending a semantic Result. Code evidence
/// requires resolve_output_facts; failed Results may omit an output entirely.
pub(crate) fn validate_report_output(
    state: &OrchestrationState, index: usize, action: &OrchestrationAction,
    resolved: Option<&ResolvedMutationInput>, session_id: &str, current_task_revision: Option<&str>,
) -> Result<(), InspectionError> {
    let OrchestrationAction::Report { message_id, kind, outcome, source_output, .. } = action else { return Ok(()) };
    if *kind != ReportKind::Result {
        if source_output.is_some() { return Err(error("invalid_source_output", "Only Result carries source output")); }
        return Ok(());
    }
    let Some(output) = source_output else {
        return if *outcome == Some(ReportOutcome::Succeeded) {
            Err(error("source_output_required", "Successful Result requires actual source output"))
        } else { Ok(()) };
    };
    validate_checks(checks(output))?;
    let run = &state.runs[index];
    let (result_id, revision) = output_identity(output);
    if result_id != message_id || revision.is_empty() {
        return Err(error("source_output_conflict", "Source output must identify this Result and the worker's current scope"));
    }
    if let Some(actual_revision) = current_task_revision {
        validate_output_task_revision(output, actual_revision)?;
    }
    match output {
        SourceOutput::NonCode { artifacts, .. } => {
            if artifacts.len() > 256 { return Err(error("invalid_source_output", "Too many output artifacts")); }
            for artifact in artifacts { bounded(artifact, 4096)?; if artifact.trim().is_empty() { return Err(error("invalid_source_output", "Artifact identity cannot be empty")); } }
        }
        SourceOutput::Code { .. } => {
            let proof = resolved.and_then(|r| r.output_facts.as_ref()).ok_or_else(|| error("source_output_unverified", "Code output requires fresh core Git observations"))?;
            validate_proof(state, run, action, session_id, proof)?;
        }
    }
    Ok(())
}

fn claim_for<'a>(state: &'a OrchestrationState, run: &Run) -> Result<&'a claims::CheckoutClaim, InspectionError> {
    let checkout = run.checkout.as_ref().ok_or_else(|| error("source_output_unverified", "Worker checkout identity is missing"))?;
    let attachment = run.attachment.as_ref().ok_or_else(|| error("source_output_unverified", "Worker attachment is missing"))?;
    let claim = state.checkout_claims.iter().find(|c| Some(c.claim_id.as_str()) == run.checkout_claim_id.as_deref())
        .ok_or_else(|| error("source_output_unverified", "Worker checkout claim is missing"))?;
    if claim.checkout != *checkout || claim.holder.as_ref().is_none_or(|h| h.run_id != run.run_id
        || h.attachment_id != attachment.attachment_id || h.attachment_epoch != attachment.epoch)
        || !matches!(claim.state, claims::ClaimState::Reserved | claims::ClaimState::Held | claims::ClaimState::Quiescing) {
        return Err(error("checkout_claim_conflict", "Source checkout is not held by this job incarnation"));
    }
    Ok(claim)
}

fn validate_proof(state: &OrchestrationState, run: &Run, action: &OrchestrationAction, session_id: &str, proof: &OutputFacts) -> Result<(), InspectionError> {
    let claim = claim_for(state, run)?;
    if proof.run_id != run.run_id || proof.request_digest != resume::request_digest(action, session_id)?
        || run.checkout.as_ref() != Some(&proof.source_checkout)
        || claim.claim_id != proof.source_claim_id || claim.revision != proof.source_claim_revision
        || claim.holder.as_ref() != Some(&proof.source_holder) {
        return Err(error("source_output_conflict", "Git observations no longer match the request or current checkout holder"));
    }
    project_identity::revalidate_checkout(&proof.source_checkout)?;
    if let Some(target) = &proof.target_checkout { project_identity::revalidate_checkout(target)?; }
    Ok(())
}

pub(in crate::orchestration) fn authorize_replay(state: &OrchestrationState, actor: &Actor, session_id: &str, run_id: &str) -> Result<usize, InspectionError> {
    let index = scoped_index(state, session_id, run_id)?;
    if let Actor::Operator(_) = actor { return Ok(index); }
    let Actor::Agent(agent) = actor else { return Err(error("actor_forbidden", "Bootstrap cannot record integration")); };
    let job = credentials::validate_job_auth(state, &agent.auth)?;
    let root = &state.runs[job.run_index];
    if job.role != JobRole::Root || root.stage != RunStage::Active || root.parent_run_id.is_some()
        || state.runs[index].root_id != root.run_id || !is_ancestor(state, &root.run_id, run_id) {
        return Err(error("actor_forbidden", "Only the current root main or operator may record accepted worker integration"));
    }
    Ok(index)
}

fn accepted_identity(ctx: &MutationCtx<'_>, record: &IntegrationRecord, index: usize) -> Result<(), InspectionError> {
    let run = &ctx.state.runs[index];
    if run.stage != RunStage::Closed || run.close_reason != Some(CloseReason::Accepted)
        || run.result.as_ref().is_none_or(|r| r.outcome != Some(ReportOutcome::Succeeded)
            || r.message_id != record.expected_result_message_id) {
        return Err(error("integration_conflict", "Integration requires this exact accepted successful Result"));
    }
    require_source_identity(run, &record.expected_result_message_id, &record.expected_task_revision)?;
    let task_id = run.task_id.as_deref().ok_or_else(|| error("task_not_found", "Accepted worker has no task"))?;
    if projection::current_task_run(ctx.state, &run.root_id, task_id).is_none_or(|r| r.run_id != run.run_id) {
        return Err(error("integration_conflict", "Accepted output is not the current canonical task attempt"));
    }
    let document = ctx.locked.tasks(&run.root_id)?;
    if !document.task(task_id)?.checked || !document.checked_matches_revision(task_id, &record.expected_task_revision)? {
        return Err(error("task_revision_conflict", "Accepted canonical task no longer matches the recorded source revision"));
    }
    Ok(())
}

pub(super) fn apply(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::IntegrationRecord { record } = &action else { unreachable!() };
    let index = authorize_replay(ctx.state, ctx.actor, ctx.session_id, &record.run_id)?;
    if let Some(result) = resume::replay(ctx, &record.operation_id, DurableOperationKind::IntegrationRecord, &action)? {
        return Ok(Applied::Document(result));
    }
    accepted_identity(ctx, record, index)?;
    let run = &ctx.state.runs[index];
    let source = run.source_output.as_ref().expect("accepted source");
    let mut retained_checks = Vec::new();
    let integration = match (&record.integration, source) {
        (Integration::Pending, _) => Integration::Pending,
        (Integration::NotApplicable { reason, result_message_id }, SourceOutput::NonCode { .. }) => {
            bounded(reason, 4096)?;
            if reason.trim().is_empty() || result_message_id != &record.expected_result_message_id {
                return Err(error("integration_conflict", "Non-code exemption requires a reason and this exact Result"));
            }
            record.integration.clone()
        }
        (Integration::Integrated { checks, .. }, SourceOutput::Code { .. }) => {
            retained_checks = checks.clone();
            let proof = ctx.resolved.and_then(|r| r.output_facts.as_ref()).ok_or_else(|| error("integration_unverified", "Integration requires core Git verification"))?;
            validate_proof(ctx.state, run, &action, ctx.session_id, proof)?;
            proof.integration.clone().ok_or_else(|| error("integration_unverified", "No integration observations were supplied"))?
        }
        _ => return Err(error("integration_conflict", "Only non-code output can be not applicable; code output requires verified integration")),
    };
    if !matches!(run.integration, Integration::Pending) && run.integration != integration {
        return Err(error("integration_conflict", "A verified integration fact cannot be overwritten by a different fact"));
    }
    let receipt = IntegrationRecordResult { operation_id: record.operation_id.clone(), run_id: record.run_id.clone(),
        integration: integration.clone(), checks: retained_checks, duplicate: false };
    let root_id = run.root_id.clone();
    ctx.state.runs[index].integration = integration;
    ctx.state.runs[index].updated_at = now();
    let result = OrchestrationActionResult::IntegrationRecorded { receipt };
    resume::record(ctx, record.operation_id.clone(), DurableOperationKind::IntegrationRecord, &action, root_id, result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

impl OrchestrationService {
    /// Host calls this for code Result / integration before mutate_resolved.
    /// Existing bounded local Git reads are the only executor; no shell, network
    /// operation or caller-provided command is run.
    pub async fn resolve_output_facts(&self, actor: &Actor, request: &OrchestrationMutationRequest) -> Result<Option<OutputFacts>, InspectionError> {
        let (state, run, source, target) = {
            let locked = self.store.lock()?;
            let state = locked.read()?;
            let (index, source, target) = match &request.action {
                OrchestrationAction::Report { kind: ReportKind::Result, source_output: Some(source @ SourceOutput::Code { .. }), .. } => {
                    let Actor::Agent(agent) = actor else { return Err(error("actor_forbidden", "Only job main reports code output")); };
                    let job = credentials::validate_job_auth(&state, &agent.auth)?;
                    if job.role == JobRole::Child { return Err(error("actor_forbidden", "Child cannot submit the main Result")); }
                    (job.run_index, source.clone(), None)
                }
                OrchestrationAction::IntegrationRecord { record } if matches!(record.integration, Integration::Integrated { .. }) => {
                    let index = authorize_replay(&state, actor, &request.session_id, &record.run_id)?;
                    let source = state.runs[index].source_output.clone().ok_or_else(|| error("source_output_required", "Accepted source is missing"))?;
                    let Integration::Integrated { target_repository_key, integrator_run_id, .. } = &record.integration else { unreachable!() };
                    let integrator = &state.runs[scoped_index(&state, &request.session_id, integrator_run_id)?];
                    if let Actor::Agent(agent) = actor {
                        if state.runs[credentials::validate_job_auth(&state, &agent.auth)?.run_index].run_id != *integrator_run_id {
                            return Err(error("actor_forbidden", "Integrator identity must be the authenticated job"));
                        }
                    }
                    let target = state.runs.iter().filter(|r| r.session_id == request.session_id && r.root_id == integrator.root_id)
                        .find(|r| r.project.as_ref().is_some_and(|p| p.key == *target_repository_key) && r.checkout.as_ref().is_some_and(|c| c.git.is_some()))
                        .and_then(|r| r.checkout.clone()).ok_or_else(|| error("integration_unverified", "Target repository has no known local checkout in the integrator's root"))?;
                    (index, source, Some(target))
                }
                _ => return Ok(None),
            };
            scope_run(&state, &request.session_id, index)?;
            let run = state.runs[index].clone();
            (state, run, source, target)
        };
        let claim = claim_for(&state, &run)?;
        let checkout = run.checkout.as_ref().expect("claim checkout");
        verify_source(&run, &source).await?;
        let integration = match (&request.action, &target) {
            (OrchestrationAction::IntegrationRecord { record }, Some(target)) => Some(verify_integration(record, &source, checkout, target).await?),
            _ => None,
        };
        project_identity::revalidate_checkout(checkout)?;
        Ok(Some(OutputFacts { request_digest: resume::request_digest(&request.action, &request.session_id)?, run_id: run.run_id,
            source_checkout: checkout.clone(), source_claim_id: claim.claim_id.clone(), source_claim_revision: claim.revision,
            source_holder: claim.holder.clone().expect("verified holder"), target_checkout: target, integration }))
    }
}

fn oid(value: &str) -> Result<(), InspectionError> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(error("invalid_git_object", "Commit IDs must be full lowercase Git object IDs"));
    }
    Ok(())
}

async fn commit(root: &Path, value: &str) -> Result<(), InspectionError> {
    oid(value)?;
    if space_git::git_line(root, &["rev-parse", "--verify", &format!("{value}^{{commit}}")]).await? != value {
        return Err(error("invalid_git_object", "Object is not the exact commit"));
    }
    Ok(())
}

async fn repository(checkout: &CheckoutIdentity) -> Result<(), InspectionError> {
    project_identity::revalidate_checkout(checkout)?;
    let git = checkout.git.as_ref().ok_or_else(|| error("integration_unverified", "Code output requires a Git checkout"))?;
    let root = Path::new(&checkout.path);
    if space_git::git_line(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await? != git.common_dir
        || space_git::git_line(root, &["rev-parse", "--absolute-git-dir"]).await? != git.worktree_git_dir
        || space_git::git_line(root, &["rev-parse", "--show-toplevel"]).await? != checkout.path {
        return Err(error("checkout_identity_changed", "Actual Git repository no longer matches this checkout identity"));
    }
    Ok(())
}

async fn ancestor(root: &Path, base: &str, head: &str) -> Result<(), InspectionError> {
    if !space_git::git_output(root, &["merge-base", "--is-ancestor", base, head]).await?.status.success() {
        return Err(error("integration_unverified", "Required Git ancestry is absent"));
    }
    Ok(())
}

async fn range(root: &Path, base: &str, head: &str) -> Result<Vec<String>, InspectionError> {
    commit(root, base).await?; commit(root, head).await?; ancestor(root, base, head).await?;
    let output = space_git::git_output(root, &["rev-list", "--reverse", "--topo-order", &format!("{base}..{head}")]).await?;
    if !output.status.success() { return Err(error("integration_unverified", "Cannot enumerate commit range")); }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| error("integration_unverified", "Invalid Git range evidence"))?;
    Ok(text.lines().map(str::to_owned).collect())
}

async fn verify_source(run: &Run, source: &SourceOutput) -> Result<(), InspectionError> {
    let SourceOutput::Code { repository_key, checkout_key, base_commit, head_commit, commits, .. } = source else {
        return Err(error("integration_conflict", "Only code output can be Git-integrated"));
    };
    validate_checks(checks(source))?;
    let project = run.project.as_ref().ok_or_else(|| error("source_output_unverified", "Source repository identity is missing"))?;
    let checkout = run.checkout.as_ref().ok_or_else(|| error("source_output_unverified", "Source checkout identity is missing"))?;
    if repository_key != &project.key || checkout_key != &checkout.key
        || run.setup.as_ref().and_then(|s| s.base_commit.as_ref()) != Some(base_commit)
        || !matches!((&project.source, &checkout.git), (ProjectSource::Git { common_dir }, Some(git)) if common_dir == &git.common_dir) {
        return Err(error("source_output_conflict", "Source must match the real repository, checkout and frozen pre-work baseline"));
    }
    repository(checkout).await?;
    let root = Path::new(&checkout.path);
    let actual = range(root, base_commit, head_commit).await?;
    if &actual != commits { return Err(error("source_output_conflict", "Source commits must enumerate the complete ordered base-to-head range")); }
    let actual_head = space_git::git_line(root, &["rev-parse", "--verify", "HEAD^{commit}"]).await?;
    // Later commits and dirty work are not part of this receipt, and are never
    // deleted or excused by recording integration of the accepted output.
    ancestor(root, head_commit, &actual_head).await
}

async fn changed_paths(root: &Path, base: &str, head: &str) -> Result<BTreeSet<String>, InspectionError> {
    let output = space_git::git_output(root, &["diff", "--no-ext-diff", "--no-textconv", "--no-renames", "--name-only", "-z", base, head, "--"]).await?;
    if !output.status.success() { return Err(error("integration_unverified", "Cannot read affected paths")); }
    output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| std::str::from_utf8(p)
        .map(str::to_owned).map_err(|_| error("integration_unverified", "Non-UTF8 affected paths cannot be mapped"))).collect()
}

async fn diff(root: &Path, base: &str, head: &str) -> Result<Vec<u8>, InspectionError> {
    let output = space_git::git_output(root, &["diff", "--no-ext-diff", "--no-textconv", "--no-renames", "--no-color", "--diff-algorithm=myers", "--no-indent-heuristic", "--ignore-submodules=none", "--no-relative", "--binary", "--full-index", "--src-prefix=a/", "--dst-prefix=b/", base, head, "--"]).await?;
    if !output.status.success() { return Err(error("integration_unverified", "Cannot compute deterministic full-range diff")); }
    Ok(output.stdout)
}

async fn mapping_range(root: &Path, commits: &[String]) -> Result<String, InspectionError> {
    if commits.is_empty() { return Err(error("integration_unverified", "Every mapping must cover a nonempty commit range")); }
    for id in commits { commit(root, id).await?; }
    let parents = space_git::git_line(root, &["rev-list", "--parents", "-n", "1", &commits[0]]).await?;
    let pieces: Vec<_> = parents.split_whitespace().collect();
    if pieces.len() != 2 { return Err(error("integration_unverified", "Mapped range requires an unambiguous single-parent base")); }
    let base = pieces[1].to_owned();
    if range(root, &base, commits.last().expect("nonempty")).await? != commits {
        return Err(error("integration_unverified", "Mapping must cover the entire contiguous ordered range, not selected commits"));
    }
    Ok(base)
}

fn mapped_paths(paths: &[String]) -> Result<BTreeSet<String>, InspectionError> {
    let mut set = BTreeSet::new();
    for path in paths {
        let p = Path::new(path);
        if path.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_)))
            || !set.insert(path.clone()) {
            return Err(error("integration_unverified", "Mapped paths must be unique explicit repository-relative paths"));
        }
    }
    Ok(set)
}

async fn verify_integration(record: &IntegrationRecord, source: &SourceOutput, checkout: &CheckoutIdentity, target: &CheckoutIdentity) -> Result<Integration, InspectionError> {
    let Integration::Integrated { operation_id, accepted_result_message_id, accepted_task_revision,
        source: reported_source, target_ref, target_commit, method, mappings, checks: target_checks, .. } = &record.integration else { unreachable!() };
    if operation_id != &record.operation_id || accepted_result_message_id != &record.expected_result_message_id
        || accepted_task_revision != &record.expected_task_revision || reported_source != source
        || output_identity(source) != (record.expected_result_message_id.as_str(), record.expected_task_revision.as_str()) {
        return Err(error("integration_conflict", "Integration must retain the exact accepted source and operation identity"));
    }
    validate_checks(target_checks)?;
    repository(target).await?;
    if !target_ref.starts_with("refs/heads/") || target_ref.contains('\0') {
        return Err(error("integration_unverified", "Target must be an explicit local branch ref"));
    }
    let target_root = Path::new(&target.path);
    if !space_git::git_output(target_root, &["check-ref-format", target_ref]).await?.status.success() {
        return Err(error("integration_unverified", "Target branch ref is malformed"));
    }
    commit(target_root, target_commit).await?;
    if space_git::git_line(target_root, &["rev-parse", "--verify", &format!("{target_ref}^{{commit}}")]).await? != *target_commit {
        return Err(error("integration_conflict", "Target branch does not point at the recorded target commit"));
    }
    let SourceOutput::Code { base_commit, head_commit, commits, .. } = source else { unreachable!() };
    let source_root = Path::new(&checkout.path);
    match method {
        IntegrationMethod::Ancestor => {
            commit(target_root, head_commit).await?;
            ancestor(target_root, head_commit, target_commit).await?;
            if !mappings.is_empty() { return Err(error("integration_unverified", "Ancestry integration does not need invented mappings")); }
        }
        IntegrationMethod::PatchEquivalent | IntegrationMethod::RecordedAdaptation => {
            if mappings.is_empty() || mappings.len() > 256 { return Err(error("integration_unverified", "Full explicit output mappings are required")); }
            let mut mapped_source = Vec::new();
            let mut mapped_target = Vec::new();
            for mapping in mappings {
                let source_base = mapping_range(source_root, &mapping.source_commits).await?;
                let target_base = mapping_range(target_root, &mapping.target_commits).await?;
                let source_end = mapping.source_commits.last().expect("mapped source");
                let target_end = mapping.target_commits.last().expect("mapped target");
                if mapped_paths(&mapping.source_paths)? != changed_paths(source_root, &source_base, source_end).await?
                    || mapped_paths(&mapping.target_paths)? != changed_paths(target_root, &target_base, target_end).await? {
                    return Err(error("integration_unverified", "Mapping must name every actual affected source and target path"));
                }
                if *method == IntegrationMethod::PatchEquivalent {
                    if diff(source_root, &source_base, source_end).await? != diff(target_root, &target_base, target_end).await? {
                        return Err(error("integration_unverified", "Mapped full-range patches are not byte-equivalent"));
                    }
                } else {
                    let explanation = mapping.explanation.as_deref().unwrap_or("");
                    bounded(explanation, 16384)?;
                    if explanation.trim().is_empty() { return Err(error("integration_unverified", "Every adapted mapping needs an explicit explanation")); }
                }
                mapped_source.extend(mapping.source_commits.iter().cloned());
                mapped_target.extend(mapping.target_commits.iter().cloned());
            }
            if &mapped_source != commits || mapped_target.iter().collect::<BTreeSet<_>>().len() != mapped_target.len() {
                return Err(error("integration_unverified", "Mappings must cover every source commit once and cannot reuse target commits"));
            }
            let target_base = mapping_range(target_root, &mapped_target).await?;
            ancestor(target_root, mapped_target.last().expect("mapped target"), target_commit).await?;
            if *method == IntegrationMethod::PatchEquivalent
                && diff(source_root, base_commit, head_commit).await? != diff(target_root, &target_base, mapped_target.last().expect("mapped target")).await? {
                return Err(error("integration_unverified", "Complete accepted output is not patch-equivalent"));
            }
        }
    }
    // These are integrator attestations for this exact immutable target, not a
    // generic merged boolean or authorization to execute reported commands.
    let all_passed = !target_checks.is_empty() && target_checks.iter().all(|c| c.outcome == CheckStatus::Passed
        && c.limitation.as_deref().is_none_or(|s| s.trim().is_empty()));
    let required: BTreeSet<_> = checks(source).iter().map(|c| c.contract.as_str()).collect();
    let performed: BTreeSet<_> = target_checks.iter().map(|c| c.contract.as_str()).collect();
    if !all_passed || !required.is_subset(&performed)
        || (*method == IntegrationMethod::RecordedAdaptation && required.is_empty()) {
        return Ok(Integration::Pending);
    }
    let mut verified = record.integration.clone();
    if let Integration::Integrated { verified_at, .. } = &mut verified { *verified_at = now(); }
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::space_git::tests::git;

    struct GitCase {
        directory: PathBuf,
        source_path: PathBuf,
        target_path: PathBuf,
        source_checkout: CheckoutIdentity,
        target_checkout: CheckoutIdentity,
        source: SourceOutput,
        target_commits: Vec<String>,
        project: ProjectIdentity,
        baseline: String,
    }
    impl Drop for GitCase {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.directory); }
    }
    impl GitCase {
        async fn new(adapted: bool) -> Self {
            let directory = std::env::temp_dir().join(format!("cockpit-integration-{}", Uuid::new_v4()));
            let target_path = directory.join("target");
            let source_path = directory.join("worker");
            std::fs::create_dir_all(&target_path).unwrap();
            git(&target_path, &["init", "-b", "main"]);
            git(&target_path, &["config", "user.email", "integration@example.test"]);
            git(&target_path, &["config", "user.name", "Integration test"]);
            std::fs::write(target_path.join("contract.txt"), "baseline\n").unwrap();
            git(&target_path, &["add", "."]);
            git(&target_path, &["commit", "-m", "baseline"]);
            let baseline = git(&target_path, &["rev-parse", "HEAD"]).trim().to_owned();
            git(&target_path, &["worktree", "add", "-b", "worker", source_path.to_str().unwrap(), &baseline]);
            let mut source_commits = Vec::new();
            for (message, body) in [("source one", "one\n"), ("source two", "two\n")] {
                std::fs::write(source_path.join("contract.txt"), body).unwrap();
                git(&source_path, &["add", "contract.txt"]);
                git(&source_path, &["commit", "-m", message]);
                source_commits.push(git(&source_path, &["rev-parse", "HEAD"]).trim().to_owned());
            }
            let mut target_commits = Vec::new();
            for (index, source) in source_commits.iter().enumerate() {
                git(&target_path, &["cherry-pick", source]);
                if adapted && index == 1 {
                    std::fs::write(target_path.join("contract.txt"), "adapted implementation\n").unwrap();
                    git(&target_path, &["add", "contract.txt"]);
                }
                git(&target_path, &["commit", "--amend", "-m", &format!("target {index}")]);
                target_commits.push(git(&target_path, &["rev-parse", "HEAD"]).trim().to_owned());
            }
            let (project, source_checkout) = project_identity::resolve_directory(&source_path, None, None).await.unwrap();
            let (_, target_checkout) = project_identity::resolve_directory(&target_path, None, None).await.unwrap();
            let source = SourceOutput::Code {
                repository_key: project.key.clone(), checkout_key: source_checkout.key.clone(),
                base_commit: baseline.clone(), head_commit: source_commits.last().unwrap().clone(),
                commits: source_commits, task_revision: "scope".into(), result_message_id: "result".into(),
                focused_checks: vec![CheckOutcome { contract: "changed contract".into(),
                    command_or_scenario: "focused contract scenario".into(), outcome: CheckStatus::Passed,
                    evidence: Some("source scenario passed".into()), limitation: None }],
            };
            Self { directory, source_path, target_path, source_checkout, target_checkout,
                source, target_commits, project, baseline }
        }

        fn record(&self, method: IntegrationMethod, status: CheckStatus) -> IntegrationRecord {
            let SourceOutput::Code { commits, .. } = &self.source else { unreachable!() };
            let operation_id = Uuid::new_v4().to_string();
            let target_commit = self.target_commits.last().unwrap().clone();
            IntegrationRecord {
                operation_id: operation_id.clone(), run_id: "worker".into(),
                expected_result_message_id: "result".into(), expected_task_revision: "scope".into(),
                integration: Integration::Integrated {
                    operation_id, accepted_result_message_id: "result".into(), accepted_task_revision: "scope".into(),
                    source: self.source.clone(), target_repository_key: self.project.key.clone(),
                    target_ref: "refs/heads/main".into(), target_commit: target_commit.clone(), method,
                    mappings: vec![IntegrationMapping {
                        source_commits: commits.clone(), target_commits: self.target_commits.clone(),
                        source_paths: vec!["contract.txt".into()], target_paths: vec!["contract.txt".into()],
                        explanation: Some("Adapted the full accepted contract to target implementation".into()),
                    }],
                    checks: vec![CheckOutcome { contract: "changed contract".into(),
                        command_or_scenario: "target contract scenario".into(), outcome: status,
                        evidence: Some(format!("Target scenario observed at {target_commit}")), limitation: None }],
                    integrator_run_id: "integrator".into(), verified_at: "caller timestamp ignored".into(),
                },
            }
        }
    }

    #[tokio::test]
    async fn full_range_patch_equivalence_rejects_missing_commits_and_unmapped_paths() {
        if crate::process_identity::machine_id().is_none() { return; }
        let case = GitCase::new(false).await;
        let record = case.record(IntegrationMethod::PatchEquivalent, CheckStatus::Passed);
        assert!(matches!(verify_integration(&record, &case.source, &case.source_checkout, &case.target_checkout).await.unwrap(),
            Integration::Integrated { method: IntegrationMethod::PatchEquivalent, .. }));
        let mut partial = record.clone();
        if let Integration::Integrated { mappings, .. } = &mut partial.integration { mappings[0].source_commits.pop(); }
        assert!(verify_integration(&partial, &case.source, &case.source_checkout, &case.target_checkout).await.is_err());
        let mut missing_path = record;
        if let Integration::Integrated { mappings, .. } = &mut missing_path.integration { mappings[0].target_paths.clear(); }
        assert!(verify_integration(&missing_path, &case.source, &case.source_checkout, &case.target_checkout).await.is_err());
    }

    #[test]
    fn successful_result_uses_current_scope_not_original_launch_revision() {
        let mut run = new_run("worker", "session", RunKind::Worker, "worker".into(), "root", Some("root".into()), Some("task".into()), 1);
        run.task_revision_at_start = Some("original launch scope".into());
        let output = SourceOutput::NonCode { result_message_id: "current-result".into(),
            task_revision: "updated canonical scope".into(), artifacts: vec!["actual analysis result".into()],
            focused_checks: Vec::new() };
        let state = OrchestrationState { runs: vec![run], ..Default::default() };
        let mut action = OrchestrationAction::Report { message_id: "current-result".into(),
            kind: ReportKind::Result, outcome: Some(ReportOutcome::Succeeded), summary: "Updated scope completed".into(),
            plan: None, source_output: Some(output), to_run_id: None,
            original_task_revision: Some("updated canonical scope".into()), original_claim_id: None };
        assert!(validate_report_output(&state, 0, &action, None, "session", Some("updated canonical scope")).is_ok());
        if let OrchestrationAction::Report { source_output: Some(SourceOutput::NonCode { task_revision, .. }), .. } = &mut action {
            *task_revision = "original launch scope".into();
        }
        assert!(validate_report_output(&state, 0, &action, None, "session", Some("updated canonical scope")).is_err());
    }

    #[tokio::test]
    async fn ancestry_requires_the_actual_source_head_and_fresh_target_ref() {
        if crate::process_identity::machine_id().is_none() { return; }
        let case = GitCase::new(false).await;
        let mut record = case.record(IntegrationMethod::Ancestor, CheckStatus::Passed);
        if let Integration::Integrated { mappings, .. } = &mut record.integration { mappings.clear(); }
        assert!(verify_integration(&record, &case.source, &case.source_checkout, &case.target_checkout).await.is_err());
        git(&case.target_path, &["merge", "--no-ff", "worker", "-m", "integrate actual source ancestry"]);
        let integrated_head = git(&case.target_path, &["rev-parse", "HEAD"]).trim().to_owned();
        if let Integration::Integrated { target_commit, checks, .. } = &mut record.integration {
            *target_commit = integrated_head.clone();
            checks[0].evidence = Some(format!("Target scenario observed at {integrated_head}"));
        }
        assert!(matches!(verify_integration(&record, &case.source, &case.source_checkout, &case.target_checkout).await.unwrap(),
            Integration::Integrated { method: IntegrationMethod::Ancestor, .. }));
        git(&case.target_path, &["commit", "--allow-empty", "-m", "advance target"]);
        assert!(verify_integration(&record, &case.source, &case.source_checkout, &case.target_checkout).await.is_err());
    }

    #[tokio::test]
    async fn adaptation_requires_full_explanation_and_passed_target_bound_checks() {
        if crate::process_identity::machine_id().is_none() { return; }
        let case = GitCase::new(true).await;
        let equivalent = case.record(IntegrationMethod::PatchEquivalent, CheckStatus::Passed);
        assert!(verify_integration(&equivalent, &case.source, &case.source_checkout, &case.target_checkout).await.is_err());
        let mut adaptation = case.record(IntegrationMethod::RecordedAdaptation, CheckStatus::Passed);
        assert!(matches!(verify_integration(&adaptation, &case.source, &case.source_checkout, &case.target_checkout).await.unwrap(),
            Integration::Integrated { method: IntegrationMethod::RecordedAdaptation, .. }));
        if let Integration::Integrated { mappings, .. } = &mut adaptation.integration { mappings[0].explanation = None; }
        assert!(verify_integration(&adaptation, &case.source, &case.source_checkout, &case.target_checkout).await.is_err());
        for status in [CheckStatus::Failed, CheckStatus::NotRun] {
            let pending = case.record(IntegrationMethod::RecordedAdaptation, status);
            assert_eq!(verify_integration(&pending, &case.source, &case.source_checkout, &case.target_checkout).await.unwrap(), Integration::Pending);
        }
        let mut wrong_target = case.record(IntegrationMethod::RecordedAdaptation, CheckStatus::Passed);
        if let Integration::Integrated { checks, .. } = &mut wrong_target.integration { checks[0].contract = "unrelated contract".into(); }
        assert_eq!(verify_integration(&wrong_target, &case.source, &case.source_checkout, &case.target_checkout).await.unwrap(), Integration::Pending);
    }

    #[tokio::test]
    async fn recorded_output_preserves_later_source_commits_and_dirty_work() {
        if crate::process_identity::machine_id().is_none() { return; }
        let case = GitCase::new(false).await;
        std::fs::write(case.source_path.join("later.txt"), "later independent commit\n").unwrap();
        git(&case.source_path, &["add", "later.txt"]);
        git(&case.source_path, &["commit", "-m", "later output not accepted"]);
        let later_head = git(&case.source_path, &["rev-parse", "HEAD"]).trim().to_owned();
        std::fs::write(case.source_path.join("draft.txt"), "preserved draft\n").unwrap();
        let mut run = new_run("worker", "session", RunKind::Worker, "worker".into(), "root", Some("root".into()), Some("task".into()), 1);
        run.project = Some(case.project.clone());
        run.checkout = Some(case.source_checkout.clone());
        run.setup = Some(SetupSummary {
            operation_id: None, generation: None, workspace_id: None,
            checkout_path: case.source_checkout.path.clone(), repository_id: None, branch: Some("worker".into()),
            base: None, base_commit: Some(case.baseline.clone()), ownership: None,
            effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None,
        });
        verify_source(&run, &case.source).await.unwrap();
        let record = case.record(IntegrationMethod::PatchEquivalent, CheckStatus::Passed);
        assert!(matches!(verify_integration(&record, &case.source, &case.source_checkout, &case.target_checkout).await.unwrap(), Integration::Integrated { .. }));
        assert_eq!(git(&case.source_path, &["rev-parse", "HEAD"]).trim(), later_head);
        assert_eq!(std::fs::read_to_string(case.source_path.join("draft.txt")).unwrap(), "preserved draft\n");
        assert_eq!(git(&case.target_path, &["rev-parse", "HEAD"]).trim(), case.target_commits.last().unwrap().as_str());
        let mut wrong_baseline = run;
        wrong_baseline.setup.as_mut().unwrap().base_commit = Some(later_head);
        assert!(verify_source(&wrong_baseline, &case.source).await.is_err());
    }
}
