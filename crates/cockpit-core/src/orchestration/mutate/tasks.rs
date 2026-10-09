use super::{Applied, MutationCtx};
use super::super::*;

pub(in crate::orchestration) fn assign(
    ctx: &mut MutationCtx<'_>,
    root_id: String,
    task_id: String,
    title: String,
    description: String,
) -> Result<Applied, InspectionError> {
    let service = ctx.service;
    let locked = ctx.locked;
    let actor = ctx.actor;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let origin = operator(actor)?;
    let result = assignments::assign(
        locked,
        state,
        session_id,
        origin,
        &root_id,
        &task_id,
        &title,
        &description,
    );
    service.revision.send_replace(state.revision);
    Ok(Applied::Published(OrchestrationMutationResponse {
        revision: state.revision,
        result: result?,
    }))
}

pub(in crate::orchestration) fn assignment_resolve(
    ctx: &mut MutationCtx<'_>,
    root_id: String,
    task_id: String,
    expected_task_revision: Option<String>,
    assign: bool,
) -> Result<Applied, InspectionError> {
    let service = ctx.service;
    let locked = ctx.locked;
    let actor = ctx.actor;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    operator(actor)?;
    let result = assignments::resolve(
        locked,
        state,
        session_id,
        &root_id,
        &task_id,
        expected_task_revision.as_deref(),
        assign,
    );
    service.revision.send_replace(state.revision);
    Ok(Applied::Published(OrchestrationMutationResponse {
        revision: state.revision,
        result: result?,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::orchestration) fn create(
    ctx: &mut MutationCtx<'_>,
    root_id: String,
    task_id: String,
    title: String,
    description: String,
    depends_on: Vec<String>,
    follow_up_of: Option<String>,
    expected_doc_revision: Option<String>,
    source_revision: Option<String>,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    task_scope(state, actor, caller, session_id, &root_id)?;
    active_task_root(state, session_id, &root_id)?;
    bounded(&title, 256)?;
    bounded(&description, 16 * 1024)?;
    tasks_md::validate_authoring_creation(&title, &description)?;
    let task_id = tasks_md::validate_uuid(&task_id)?.to_string();
    let edges = normalize_task_edges(&depends_on)?;
    let follow = follow_up_of.as_deref().map(tasks_md::validate_uuid)
        .transpose()?.map(|id| id.to_string());
    if !edges.is_empty() || follow.is_some() {
        task_root_authority(state, actor, caller, session_id, &root_id)?;
        if expected_doc_revision.is_none() || (follow.is_some() && source_revision.is_none()) {
            return Err(error("invalid_task", "Relationships require document and follow-up source fences"));
        }
    }
    if follow.is_none() && source_revision.is_some() {
        return Err(error("invalid_task", "Source revision requires follow_up_of"));
    }
    let document = locked.tasks(&root_id)?;
    // Only exact intended identity replay may bypass now-stale graph/source fences.
    // Shape and fresh caller authority have already been checked.
    let replay = document.tasks.iter().any(|task| task.task_id == task_id);
    if !replay {
        if expected_doc_revision.as_deref().is_some_and(|revision| revision != document.doc_revision) {
            return Err(error("task_revision_conflict", "Task document changed"));
        }
        dependencies::validate_dependencies(&document.tasks, &task_id, &edges)?;
        if let Some(source) = &follow {
            if source == &task_id {
                return Err(error("task_relations_invalid", "A task cannot follow itself"));
            }
            let source_task = document.task(source)?;
            if source_revision.as_deref() != Some(source_task.task_revision.as_str()) {
                return Err(error("task_revision_conflict", "Follow-up source changed"));
            }
        }
    }
    Ok(Applied::Document(OrchestrationActionResult::Task {
        task: document.create_authoring_with_id(&task_id, &title, &description,
            &edges, follow.as_deref(), expected_doc_revision.as_deref(), source_revision.as_deref())?,
    }))
}

pub(in crate::orchestration) fn update(
    ctx: &mut MutationCtx<'_>,
    root_id: String,
    task_id: String,
    expected_task_revision: String,
    title: Option<String>,
    description: Option<String>,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let document = locked.tasks(&root_id)?;
    task_content_target(state, actor, caller, session_id, &root_id, &task_id, &document)?;
    if let Some(t) = &title { bounded(t, 256)?; }
    if let Some(d) = &description { bounded(d, 16 * 1024)?; }
    Ok(Applied::Document(OrchestrationActionResult::Task {
        task: document.update(&task_id, &expected_task_revision, title.as_deref(), description.as_deref())?,
    }))
}

pub(in crate::orchestration) fn dependencies_set(
    ctx: &mut MutationCtx<'_>,
    root_id: String,
    task_id: String,
    expected_task_revision: String,
    expected_doc_revision: String,
    depends_on: Vec<String>,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    task_root_authority(state, actor, caller, session_id, &root_id)?;
    let document = locked.tasks(&root_id)?;
    let task = mutable_task(state, &root_id, &task_id, &document)?;
    if task.task_revision != expected_task_revision || document.doc_revision != expected_doc_revision {
        return Err(error("task_revision_conflict", "Task or document changed"));
    }
    let edges = normalize_task_edges(&depends_on)?;
    let live = state.runs.iter().any(|run| run.root_id == root_id
        && run.task_id.as_deref() == Some(task.task_id.as_str()) && run.stage != RunStage::Closed);
    if live {
        if task.relations_diagnostic.is_some() || edges.len() >= task.depends_on.len()
            || edges.iter().any(|edge| !task.depends_on.contains(edge)) {
            return Err(error("task_relationships_live", "Open attempts allow only strictly removing positively parsed prerequisites"));
        }
    } else {
        dependencies::validate_dependencies(&document.tasks, &task.task_id, &edges)?;
    }
    Ok(Applied::Document(OrchestrationActionResult::Task {
        task: document.set_dependencies(&task_id, &expected_task_revision, &expected_doc_revision, &edges)?,
    }))
}

pub(in crate::orchestration) fn assign_ids(
    ctx: &mut MutationCtx<'_>,
    root_id: String,
    expected_doc_revision: String,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    task_scope(state, actor, caller, session_id, &root_id)?;
    let (assigned, doc_revision) =
        locked.tasks(&root_id)?.assign_ids(&expected_doc_revision)?;
    Ok(Applied::Document(OrchestrationActionResult::TaskIds {
        assigned,
        doc_revision,
    }))
}

pub(in crate::orchestration) fn step<'a>(
    ctx: &mut MutationCtx<'_>,
    root_id: &str,
    task_id: &str,
    expected_task_revision: &str,
    intent: impl FnOnce() -> Result<steps::StepIntent<'a>, InspectionError>,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let document = locked.tasks(root_id)?;
    task_content_target(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id, task_id, &document)?;
    Ok(Applied::Document(OrchestrationActionResult::Task {
        task: document.step(task_id, expected_task_revision, intent()?)?,
    }))
}

pub(super) fn normalize_task_edges(edges: &[String]) -> Result<Vec<String>, InspectionError> {
    if edges.len() > 32 {
        return Err(error("task_relations_invalid", "At most 32 prerequisites are allowed"));
    }
    let mut normalized = edges.iter().map(|edge| tasks_md::validate_uuid(edge).map(|id| id.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort_unstable();
    normalized.dedup();
    Ok(normalized)
}

pub(super) fn active_task_root<'a>(
    state: &'a OrchestrationState, session: &str, root_id: &str,
) -> Result<&'a Run, InspectionError> {
    let root = &state.runs[scoped_index(state, session, root_id)?];
    if root.parent_run_id.is_some() || root.root_id != root.run_id
        || !matches!(root.kind, RunKind::Supervisor | RunKind::Adopted) {
        return Err(error("root_not_found", "Expected a supervisor root"));
    }
    require_stage(root, RunStage::Active)?;
    Ok(root)
}

/// The host CLI supplies fresh native process evidence, never action JSON.
pub(super) fn native_task_caller(run: &Run, agent: &AgentCaller) -> bool {
    agent.actual_agent_kind.as_ref() == Some(&NativeAgentKind::Omp)
        && caller_matches(run, agent) && session_matches(run, agent)
        && run.bound_omp_session.as_deref().is_some_and(|session| !session.is_empty())
        && run.bound_omp_process.as_ref().is_some_and(|process| {
            process.kernel_boot_id.as_deref().is_some_and(|boot| !boot.is_empty())
                && agent.process.as_ref() == Some(process)
        })
        && (run.kind == RunKind::Adopted && run.dispatch.is_none()
            || run.dispatch.as_ref().is_some_and(|dispatch| dispatch.agent_started))
        && !dispatch::recovery_incarnation_revoked(run)
}

pub(super) fn task_root_authority(
    state: &OrchestrationState, actor: &Actor, caller: Option<usize>, session: &str, root_id: &str,
) -> Result<(), InspectionError> {
    let root = active_task_root(state, session, root_id)?;
    if matches!(actor, Actor::Operator(_)) { return Ok(()) }
    let Actor::Agent(agent) = actor else { unreachable!() };
    if state.runs[required_caller(caller)?].run_id != root.run_id
        || agent.agent_kind != Some(AgentKind::Main) || agent.subagent_id.is_some()
        || !native_task_caller(root, agent) {
        return Err(error("actor_forbidden", "Only the bound main root or operator may author relationships"));
    }
    Ok(())
}

pub(super) fn mutable_task<'a>(
    state: &OrchestrationState, root_id: &str, task_id: &str, document: &'a tasks_md::TaskDocument<'_>,
) -> Result<&'a Task, InspectionError> {
    let task = document.task(task_id)?;
    if task.checked { return Err(error("task_checked", "Accepted canonical tasks are read-only")) }
    if state.task_intents.iter().any(|intent| intent.root_id == root_id && intent.task_id == task.task_id) {
        return Err(error("intent_conflict", "Resolve the existing acceptance intent first"));
    }
    Ok(task)
}

pub(super) fn task_content_target(
    state: &OrchestrationState, actor: &Actor, caller: Option<usize>, session: &str,
    root_id: &str, task_id: &str, document: &tasks_md::TaskDocument<'_>,
) -> Result<(), InspectionError> {
    active_task_root(state, session, root_id)?;
    let task = mutable_task(state, root_id, task_id, document)?;
    let current = projection::current_task_run(state, root_id, &task.task_id);
    if matches!(actor, Actor::Operator(_))
        || caller.is_some_and(|index| state.runs[index].run_id == root_id) {
        task_root_authority(state, actor, caller, session, root_id)?;
        if current.is_some_and(|run| run.stage == RunStage::Working) {
            return Err(error("actor_forbidden", "Only the current executed worker may edit its own canonical task"));
        }
        return Ok(());
    }
    let Actor::Agent(agent) = actor else { unreachable!() };
    let run = &state.runs[required_caller(caller)?];
    if run.root_id != root_id {
        return Err(error("actor_forbidden", "Caller does not own this canonical root"));
    }
    if run.kind != RunKind::Worker || run.stage != RunStage::Working
        || run.task_id.as_deref() != Some(task.task_id.as_str())
        || current.is_none_or(|current| current.run_id != run.run_id)
        || run.init_receipt.as_ref().is_none_or(|receipt| receipt.kind != ReportKind::Ready)
        || run.work_plan.as_ref().is_none_or(|plan| {
            run.init_receipt.as_ref().and_then(|receipt| receipt.plan.as_deref()) != Some(plan.text.as_str())
                || !run.grants.iter().any(|grant| {
                    grant.scope == GrantScope::Execute && grant.plan_revision == plan.plan_revision
                })
        })
        || !native_task_caller(run, agent) {
        return Err(error("actor_forbidden", "Only the current executed worker may edit its own canonical task"));
    }
    match agent.agent_kind {
        Some(AgentKind::Main) if agent.subagent_id.is_none() => {}
        Some(AgentKind::Subagent) => {
            let session = messages::authenticated_child_session(agent, run)?;
            if !state.subagents.iter().any(|child| {
                child.run_id == run.run_id && Some(child.subagent_id.as_str()) == agent.subagent_id.as_deref()
                    && child.status == SubagentStatus::Running
                    && child.bound_omp_session.as_deref() == Some(session)
            }) {
                return Err(error("actor_forbidden", "An authenticated live native child binding is required"));
            }
        }
        _ => return Err(error("actor_forbidden", "An authenticated live native child binding is required")),
    }
    dependencies::DependencyGraph::new(&document.tasks).require(task)
}

pub(super) fn task_transition_target(
    state: &OrchestrationState, run: &Run, document: &tasks_md::TaskDocument<'_>, allow_replacement: bool,
) -> Result<(), InspectionError> {
    active_task_root(state, &run.session_id, &run.root_id)?;
    let task_id = run.task_id.as_deref().ok_or_else(|| error("task_not_found", "Worker task is missing"))?;
    let task = mutable_task(state, &run.root_id, task_id, document)?;
    let current = projection::current_task_run(state, &run.root_id, &task.task_id);
    let selected = current.is_some_and(|current| current.run_id == run.run_id);
    let pending = allow_replacement && matches!(run.stage, RunStage::Proposed | RunStage::AwaitingPrepare)
        && current.is_some_and(|current| run.supersedes_run_id.as_deref() == Some(current.run_id.as_str()))
        && !state.runs.iter().any(|other| other.root_id == run.root_id
            && other.task_id == run.task_id && other.stage != RunStage::Closed && other.attempt > run.attempt);
    if !selected && !pending {
        return Err(error("attempt_stale", "Worker no longer owns the current canonical task attempt"));
    }
    dependencies::DependencyGraph::new(&document.tasks).require(task)
}

pub(super) fn task_scope(
    state: &OrchestrationState,
    actor: &Actor,
    caller: Option<usize>,
    session: &str,
    root: &str,
) -> Result<(), InspectionError> {
    let index = scoped_index(state, session, root)?;
    if state.runs[index].parent_run_id.is_some() {
        return Err(error("root_not_found", "Expected a supervisor root"));
    }
    if matches!(actor, Actor::Agent(_)) && state.runs[required_caller(caller)?].root_id != root {
        return Err(error(
            "actor_forbidden",
            "Agent may only edit canonical tasks in its own root",
        ));
    }
    Ok(())
}
