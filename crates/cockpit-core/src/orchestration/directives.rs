use super::*;
use super::mutate::{Applied, MutationCtx, resume};
use super::store::DurableOperationKind;

fn applies(directive: &Directive, run: &Run) -> bool {
    directive.removed_at.is_none() && directive.root_id == run.root_id && match &directive.scope {
        DirectiveScope::Task { task_id } => run.task_id.as_deref() == Some(task_id),
        DirectiveScope::Standing { project_key } => project_key.as_ref().is_none_or(|key|
            run.project.as_ref().is_some_and(|project| &project.key == key)),
    }
}

pub fn active_for_run<'a>(state: &'a OrchestrationState, run: &Run) -> Vec<&'a Directive> {
    state.directives.iter().filter(|directive| applies(directive, run)).collect()
}

pub fn active_for_root<'a>(state: &'a OrchestrationState, root_id: &str) -> Vec<&'a Directive> {
    state.directives.iter().filter(|directive| directive.root_id == root_id && directive.removed_at.is_none()).collect()
}

impl OrchestrationService {
    pub fn job_directives(&self, caller: &AgentCaller) -> Result<Vec<Directive>, InspectionError> {
        let state = self.store.lock()?.read_shared()?;
        let job = credentials::validate_caller(&state, caller)?;
        let run = &state.runs[job.run_index];
        let directives = if job.role == JobRole::Root { active_for_root(&state, &run.root_id) }
            else { active_for_run(&state, run) };
        Ok(directives.into_iter().cloned().collect())
    }
}

fn validate_record(state: &OrchestrationState, directive: &Directive) -> Result<(), InspectionError> {
    tasks_md::validate_uuid(&directive.directive_id)?;
    if directive.text.trim().is_empty() { return Err(error("invalid_message", "Directive text must not be empty")); }
    bounded(&directive.text, 4096)?;
    if state.directives.iter().any(|existing| existing.directive_id == directive.directive_id) {
        return Err(error("operation_conflict", "Directive ID is already in use"));
    }
    if state.directives.iter().filter(|existing| existing.root_id == directive.root_id).count() >= 256 {
        return Err(error("directive_limit", "A root may have at most 256 directive records"));
    }
    if matches!(directive.scope, DirectiveScope::Standing { .. }) && state.directives.iter().filter(|existing|
        existing.root_id == directive.root_id && existing.removed_at.is_none()
            && matches!(existing.scope, DirectiveScope::Standing { .. })).count() >= 32 {
        return Err(error("directive_limit", "A root may have at most 32 active standing directives"));
    }
    Ok(())
}

pub(super) fn record_task_directive(state: &mut OrchestrationState, root_id: &str, task_id: &str,
    text: &str, thread_id: &str, message_id: &str) -> Result<String, InspectionError> {
    let directive_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, message_id.as_bytes()).to_string();
    let directive = Directive { directive_id: directive_id.clone(), root_id: root_id.to_owned(),
        scope: DirectiveScope::Task { task_id: task_id.to_owned() }, text: text.to_owned(),
        source: DirectiveSource::Inbox, thread_id: Some(thread_id.to_owned()), message_id: Some(message_id.to_owned()),
        created_by: ActorRef::Operator, created_at: now(), removed_at: None };
    validate_record(state, &directive)?;
    state.directives.push(directive);
    Ok(directive_id)
}

pub(in crate::orchestration) fn add(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::DirectiveAdd { directive_id, root_id, scope, text, thread_id } = &action else { unreachable!() };
    if let Some(mut result) = resume::replay(ctx, directive_id, DurableOperationKind::DirectiveAdd, &action)? {
        if let OrchestrationActionResult::DirectiveAdded { duplicate, .. } = &mut result { *duplicate = true; }
        return Ok(Applied::Document(result));
    }
    mutate::tasks::task_root_authority(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id)?;
    if let DirectiveScope::Task { task_id } = scope { ctx.locked.tasks(root_id)?.task(task_id)?; }
    if let Some(thread_id) = thread_id { threads::validate_thread_id(thread_id)?; }
    let directive = Directive { directive_id: directive_id.clone(), root_id: root_id.clone(), scope: scope.clone(),
        text: text.clone(), source: if matches!(ctx.actor, Actor::Operator(_)) { DirectiveSource::Operator } else { DirectiveSource::Supervisor },
        thread_id: thread_id.clone(), message_id: None, created_by: actor_ref(ctx.actor, ctx.caller, ctx.state),
        created_at: now(), removed_at: None };
    validate_record(ctx.state, &directive)?;
    let targets: Vec<String> = ctx.state.runs.iter().filter(|run| run.kind == RunKind::Worker && run.stage != RunStage::Closed
        && applies(&directive, run)).map(|run| run.run_id.clone()).collect();
    for run_id in &targets {
        let id = format!("directive:{directive_id}:{run_id}");
        messages::append(ctx.state, messages::AppendMessage { from: directive.created_by.clone(), to_run_id: run_id,
            message_id: &id, kind: MessageKind::UserDirective, text, in_reply_to: None, report: None,
            thread_id: thread_id.clone(), handoff: None, images: Vec::new(), stale: false, escalated_from: None })?;
    }
    ctx.state.directives.push(directive);
    let result = OrchestrationActionResult::DirectiveAdded { directive_id: directive_id.clone(), delivered_run_ids: targets, duplicate: false };
    resume::record(ctx, directive_id.clone(), DurableOperationKind::DirectiveAdd, &action, root_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(in crate::orchestration) fn remove(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::DirectiveRemove { root_id, directive_id } = action else { unreachable!() };
    mutate::tasks::task_root_authority(ctx.state, ctx.actor, ctx.caller, ctx.session_id, &root_id)?;
    let directive = ctx.state.directives.iter_mut().find(|directive| directive.root_id == root_id && directive.directive_id == directive_id)
        .ok_or_else(|| error("directive_not_found", "Directive not found for this root"))?;
    let result = OrchestrationActionResult::DirectiveRemoved { directive_id };
    if directive.removed_at.is_some() { return Ok(Applied::Document(result)); }
    directive.removed_at = Some(now());
    Ok(Applied::Machine(result))
}
