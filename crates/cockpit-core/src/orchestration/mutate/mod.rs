use super::*;
use super::store::LockedStore;

pub(super) mod tasks;
pub(super) mod runs;
pub(super) mod resume;
pub(super) mod grants;
mod ownership;
pub(super) mod intents;
pub(super) mod reviewed;
mod dispatch_records;
pub(super) mod integration;

/// The mutation's existing lock and caller admission, borrowed by each handler.
/// Timestamps stay at their original call sites rather than being cached here.
pub(in crate::orchestration) struct MutationCtx<'a> {
    pub(in crate::orchestration) service: &'a OrchestrationService,
    pub(in crate::orchestration) locked: &'a LockedStore<'a>,
    pub(in crate::orchestration) state: &'a mut OrchestrationState,
    pub(in crate::orchestration) actor: &'a Actor,
    pub(in crate::orchestration) caller: Option<usize>,
    pub(in crate::orchestration) session_id: &'a str,
    pub(in crate::orchestration) resolved: Option<&'a ResolvedMutationInput>,
}

pub(in crate::orchestration) enum Applied {
    Machine(OrchestrationActionResult),
    Document(OrchestrationActionResult),
    Published(OrchestrationMutationResponse),
}

pub(super) fn apply(
    ctx: &mut MutationCtx<'_>,
    action: OrchestrationAction,
) -> Result<Applied, InspectionError> {
    match action {
        OrchestrationAction::TaskAssign { root_id, task_id, title, description } =>
            tasks::assign(ctx, root_id, task_id, title, description),
        OrchestrationAction::TaskAssignmentResolve { root_id, task_id, expected_task_revision, assign } =>
            tasks::assignment_resolve(ctx, root_id, task_id, expected_task_revision, assign),
        OrchestrationAction::TaskCreate {
            root_id, task_id, title, description, depends_on, follow_up_of,
            expected_doc_revision, source_revision,
        } => tasks::create(ctx, root_id, task_id, title, description, depends_on,
            follow_up_of, expected_doc_revision, source_revision),
        OrchestrationAction::TaskUpdate { root_id, task_id, expected_task_revision, title, description } =>
            tasks::update(ctx, root_id, task_id, expected_task_revision, title, description),
        OrchestrationAction::TaskDependenciesSet {
            root_id, task_id, expected_task_revision, expected_doc_revision, depends_on,
        } => tasks::dependencies_set(ctx, root_id, task_id, expected_task_revision,
            expected_doc_revision, depends_on),
        OrchestrationAction::TaskStepAdd {
            root_id, task_id, expected_task_revision, step_id, parent_step_id, before_step_id, title,
        } => tasks::step(ctx, &root_id, &task_id, &expected_task_revision, || {
            Ok(steps::StepIntent::Add {
                step_id: tasks_md::validate_uuid(&step_id)?,
                parent_step_id: parent_step_id.as_deref().map(tasks_md::validate_uuid).transpose()?,
                before_step_id: before_step_id.as_deref().map(tasks_md::validate_uuid).transpose()?,
                title: &title,
            })
        }),
        OrchestrationAction::TaskStepRename {
            root_id, task_id, expected_task_revision, step_id, title,
        } => tasks::step(ctx, &root_id, &task_id, &expected_task_revision, || {
            Ok(steps::StepIntent::Rename { step_id: tasks_md::validate_uuid(&step_id)?, title: &title })
        }),
        OrchestrationAction::TaskStepSetChecked {
            root_id, task_id, expected_task_revision, step_id, checked, scope,
        } => tasks::step(ctx, &root_id, &task_id, &expected_task_revision, || {
            Ok(steps::StepIntent::SetChecked { step_id: tasks_md::validate_uuid(&step_id)?, checked, scope })
        }),
        OrchestrationAction::TaskStepMove {
            root_id, task_id, expected_task_revision, step_id, parent_step_id, before_step_id,
        } => tasks::step(ctx, &root_id, &task_id, &expected_task_revision, || {
            Ok(steps::StepIntent::Move {
                step_id: tasks_md::validate_uuid(&step_id)?,
                parent_step_id: parent_step_id.as_deref().map(tasks_md::validate_uuid).transpose()?,
                before_step_id: before_step_id.as_deref().map(tasks_md::validate_uuid).transpose()?,
            })
        }),
        OrchestrationAction::TaskStepRemove { root_id, task_id, expected_task_revision, step_id } =>
            tasks::step(ctx, &root_id, &task_id, &expected_task_revision, || {
                Ok(steps::StepIntent::Remove { step_id: tasks_md::validate_uuid(&step_id)? })
            }),
        OrchestrationAction::TasksAssignIds { root_id, expected_doc_revision } =>
            tasks::assign_ids(ctx, root_id, expected_doc_revision),
        action @ OrchestrationAction::SupervisorStart { .. } =>
            runs::supervisor_start(ctx, action),
        action @ OrchestrationAction::RunResume { .. } =>
            resume::resume(ctx, action),
        action @ OrchestrationAction::RunAttach { .. } =>
            resume::attach(ctx, action),
        action @ OrchestrationAction::RunRegister { .. } => ownership::register(ctx, action),
        action @ OrchestrationAction::TaskUnassign { .. } => ownership::unassign(ctx, action),
        OrchestrationAction::RunAdopt { label, location } => runs::adopt(ctx, label, location),
        action @ OrchestrationAction::WorkerStart { .. } => runs::worker_start(ctx, action),
        action @ OrchestrationAction::TaskScopeUpdate { .. } => tasks::scope_update(ctx, action),
        action @ OrchestrationAction::IntegrationRecord { .. } => integration::apply(ctx, action),
        action @ (OrchestrationAction::ResourceRequest { .. }
            | OrchestrationAction::ResourceRelease { .. }
            | OrchestrationAction::ResourceBuildRegister { .. }) =>
            super::resource_queue::apply(ctx, action),
        action @ (OrchestrationAction::ResourceRegister { .. } | OrchestrationAction::CleanupFinish { .. }) =>
            super::cleanup::apply(ctx, action),
        OrchestrationAction::Accept { run_id, expected_task_revision } =>
            intents::accept(ctx, run_id, expected_task_revision),
        OrchestrationAction::SendBack { run_id, text } =>
            grants::send_back(ctx, run_id, text),
        OrchestrationAction::CancelRun { run_id } => runs::cancel(ctx, run_id),
        OrchestrationAction::ReconcileRun { run_id, recovery } =>
            runs::reconcile(ctx, run_id, recovery),
        OrchestrationAction::IntentResolve { intent_id, apply } =>
            intents::resolve(ctx, intent_id, apply),
        _ => Err(error("actor_forbidden", "Unsupported actor/action combination")),
    }
}
