//! Fresh Git containment for removal only; never target-check or Integration evidence.
use super::*;

#[derive(Debug, Clone)]
pub(crate) struct MergedProof {
    pub(crate) target_run_id: String,
    pub(crate) target_checkout_key: String,
    pub(crate) target_commit: String,
    pub(crate) accepted_head: String,
    target_checkout: CheckoutIdentity,
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
        self.matches_run(run) && state.runs.iter().any(|target| target.run_id == self.target_run_id
            && target.root_id == target.run_id && target.session_id == run.session_id
            && target.project.as_ref().is_some_and(|project| project.key == self.repository_key)
            && target.checkout.as_ref() == Some(&self.target_checkout))
    }

    pub(crate) async fn reprove(&self) -> Result<(), InspectionError> {
        if crate::project_adapter::owned_git::contained_head(&self.target_checkout, &self.accepted_head,
            Some(&self.target_commit)).await?.is_none() {
            return Err(error("merged_head_unproven", "Accepted head is no longer contained in the root checkout target"));
        }
        Ok(())
    }

    pub(crate) fn evidence(&self) -> String {
        format!("merged into root {} at {}; non-force removal", self.target_run_id, self.target_commit)
    }
}

pub(crate) async fn merged_proof(state: &OrchestrationState, run: &Run) -> Result<Option<MergedProof>, InspectionError> {
    let Some(source @ SourceOutput::Code { repository_key, head_commit, .. }) = run.source_output.as_ref() else { return Ok(None) };
    let Some(root) = state.runs.iter().find(|root| root.run_id == run.root_id
        && root.root_id == root.run_id && root.session_id == run.session_id) else { return Ok(None) };
    let (Some(project), Some(target)) = (&root.project, &root.checkout) else { return Ok(None) };
    if project.key != *repository_key || target.git.is_none()
        || run.project.as_ref().is_none_or(|source_project| source_project.key != project.key)
        || run.checkout.as_ref().is_none_or(|checkout| checkout.key == target.key
            || checkout.git.as_ref().is_none_or(|git| target.git.as_ref().is_none_or(|target_git| git.common_dir != target_git.common_dir))) {
        return Ok(None);
    }
    let Some(target_commit) = crate::project_adapter::owned_git::contained_head(target, head_commit, None).await? else {
        return Ok(None);
    };
    let proof = MergedProof { target_run_id: root.run_id.clone(), target_checkout_key: target.key.clone(),
        target_commit, accepted_head: head_commit.clone(), target_checkout: target.clone(),
        repository_key: repository_key.clone(), source_run_id: run.run_id.clone(), source: source.clone() };
    if !proof.matches_state(state, run) { return Ok(None) }
    Ok(Some(proof))
}
