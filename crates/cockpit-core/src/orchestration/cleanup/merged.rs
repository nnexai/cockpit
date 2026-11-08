//! Fresh Git containment for removal only; never target-check or Integration evidence.
use super::*;

#[derive(Debug, Clone, Copy)]
enum ContainmentMethod { Ancestor, PatchEquivalent }

#[derive(Debug, Clone)]
pub(crate) struct MergedProof {
    target_run_id: String,
    target_checkout_key: String,
    target_commit: String,
    pub(crate) accepted_head: String,
    target_checkout: CheckoutIdentity,
    method: ContainmentMethod,
    repository_key: String,
    source_run_id: String,
    source: SourceOutput,
}

impl MergedProof {
    pub(crate) fn matches_run(&self, run: &Run) -> bool {
        run.run_id == self.source_run_id && run.root_id == self.target_run_id
            && run.kind == RunKind::Worker && run.stage == RunStage::Closed
            && run.close_reason == Some(CloseReason::Accepted)
            && run.project.as_ref().is_some_and(|project| project.key == self.repository_key)
            && run.checkout.as_ref().is_some_and(|checkout| checkout.key != self.target_checkout_key
                && matches!(&self.source, SourceOutput::Code { checkout_key, .. } if checkout_key == &checkout.key))
            && run.source_output.as_ref() == Some(&self.source)
            && run.result.as_ref().is_some_and(|result| result.outcome == Some(ReportOutcome::Succeeded)
                && result.source_output.as_ref() == Some(&self.source)
                && matches!(&self.source, SourceOutput::Code { result_message_id, head_commit, .. }
                    if result_message_id == &result.message_id && head_commit == &self.accepted_head))
    }

    pub(crate) fn matches_state(&self, state: &OrchestrationState, run: &Run) -> bool {
        self.matches_run(run) && state.runs.iter().any(|root| root.run_id == self.target_run_id
            && root.root_id == root.run_id && root.session_id == run.session_id)
    }

    pub(crate) async fn reprove(&self) -> Result<(), InspectionError> {
        let SourceOutput::Code { base_commit, head_commit, commits, .. } = &self.source else { unreachable!() };
        let proof = match self.method {
            ContainmentMethod::Ancestor => crate::project_adapter::owned_git::contained_head(
                &self.target_checkout, head_commit, Some(&self.target_commit)).await?,
            ContainmentMethod::PatchEquivalent => crate::project_adapter::owned_git::patch_contained(
                &self.target_checkout, base_commit, head_commit, commits, Some(&self.target_commit)).await?,
        };
        if proof.is_none() {
            return Err(error("merged_head_unproven", "Accepted output is no longer contained in the project's primary checkout"));
        }
        Ok(())
    }

    pub(crate) fn evidence(&self) -> String {
        let method = match self.method { ContainmentMethod::Ancestor => "ancestor", ContainmentMethod::PatchEquivalent => "patch-equivalent" };
        format!("{method} containment in {} at {}; non-force removal", self.target_checkout.path, self.target_commit)
    }
}

pub(crate) async fn merged_proof(state: &OrchestrationState, run: &Run) -> Result<Option<MergedProof>, InspectionError> {
    let Some(source @ SourceOutput::Code { repository_key, base_commit, head_commit, commits, .. }) = run.source_output.as_ref() else { return Ok(None) };
    let Some(worker) = run.checkout.as_ref() else { return Ok(None) };
    let Some(git) = worker.git.as_ref() else { return Ok(None) };
    let Some(path) = crate::project_adapter::owned_git::primary_worktree(&git.common_dir).await? else { return Ok(None) };
    let (project, target) = project_identity::resolve_directory(std::path::Path::new(&path), None, None).await?;
    if project.key != *repository_key || target.key == worker.key
        || target.git.as_ref().is_none_or(|target_git| target_git.common_dir != git.common_dir)
        || run.project.as_ref().is_none_or(|source_project| source_project.key != project.key) {
        return Ok(None);
    }
    let (method, target_commit) = if let Some(head) = crate::project_adapter::owned_git::contained_head(&target, head_commit, None).await? {
        (ContainmentMethod::Ancestor, head)
    } else if let Some(head) = crate::project_adapter::owned_git::patch_contained(&target, base_commit, head_commit, commits, None).await? {
        (ContainmentMethod::PatchEquivalent, head)
    } else { return Ok(None) };
    let proof = MergedProof { target_run_id: run.root_id.clone(), target_checkout_key: target.key.clone(),
        target_commit, accepted_head: head_commit.clone(), target_checkout: target, method,
        repository_key: repository_key.clone(), source_run_id: run.run_id.clone(), source: source.clone() };
    if !proof.matches_state(state, run) { return Ok(None) }
    Ok(Some(proof))
}
