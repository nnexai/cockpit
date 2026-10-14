use super::*;

impl OrchestrationService {
    /// Return only accepted outputs pinned to this exact canonical document.
    /// Missing, unchecked or manually checked prerequisites supply no accepted
    /// worker output identity; readiness remains a separate scheduling gate.
    pub fn accepted_dependency_run_ids(
        &self,
        session: &str,
        root: &str,
        task: &str,
        expected_doc_revision: &str,
    ) -> Result<Vec<String>, InspectionError> {
        validate_identity(session)?;
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let root_run = &state.runs[scoped_index(&state, session, root)?];
        if root_run.parent_run_id.is_some()
            || root_run.root_id != root_run.run_id
            || !matches!(root_run.kind, RunKind::Supervisor | RunKind::Adopted)
        {
            return Err(error("root_not_found", "Expected a supervisor root"));
        }
        let document = locked.tasks(root)?;
        if document.doc_revision != expected_doc_revision {
            return Err(error(
                "task_revision_conflict",
                "Task document changed; refresh before reading dependency outputs",
            ));
        }
        let target = document.task(task)?;
        let mut accepted = Vec::with_capacity(target.depends_on.len());
        for dependency in &target.depends_on {
            if let Some(run) = accepted_dependency_run(
                &state, &document, session, root, dependency,
            )? {
                accepted.push(run.run_id.clone());
            }
        }
        Ok(accepted)
    }
}

/// Shared by readonly inputs and launch eligibility: a checkbox is canonical
/// completion, not evidence that an arbitrary historical output was accepted.
pub(super) fn accepted_dependency_run<'a>(
    state: &'a OrchestrationState,
    document: &tasks_md::TaskDocument<'_>,
    session: &str,
    root: &str,
    task: &str,
) -> Result<Option<&'a Run>, InspectionError> {
    let prerequisite = match document.task(task) {
        Ok(task) => task,
        Err(failure) if matches!(failure.code.as_str(),
            "task_not_found" | "task_id_duplicate" | "invalid_identity") => return Ok(None),
        Err(failure) => return Err(failure),
    };
    if !prerequisite.checked {
        return Ok(None);
    }
    let Some(run) = projection::current_task_run(state, root, &prerequisite.task_id) else {
        return Ok(None);
    };
    if run.session_id != session
        || run.root_id != root
        || run.task_id.as_deref() != Some(prerequisite.task_id.as_str())
    {
        return Err(error("source_output_conflict", "Prerequisite output belongs to another task, root or session"));
    }
    if run.stage != RunStage::Closed || run.close_reason != Some(CloseReason::Accepted) {
        return Ok(None);
    }
    let Some(result) = &run.result else {
        if run.source_output.is_some() {
            return Err(error("source_output_conflict", "Prerequisite source output has no accepted Result"));
        }
        return Ok(None);
    };
    if run.kind != RunKind::Worker
        || result.kind != ReportKind::Result
        || result.outcome != Some(ReportOutcome::Succeeded)
    {
        return Ok(None);
    }
    let Some(source) = run.source_output.as_ref() else {
        return Ok(None);
    };
    let (result_id, revision) = mutate::integration::output_identity(source);
    if result_id != result.message_id || revision.is_empty() {
        return Err(error("source_output_conflict", "Prerequisite source output does not identify its accepted Result and scope"));
    }
    mutate::integration::require_source_identity(run, &result.message_id, revision)?;
    if !document.checked_matches_revision(&prerequisite.task_id, revision)? {
        return Err(error("task_revision_conflict", "Accepted prerequisite scope changed in the canonical task document"));
    }
    Ok(Some(run))
}
