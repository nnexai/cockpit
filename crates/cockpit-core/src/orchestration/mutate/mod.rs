use super::*;
use super::store::LockedStore;

pub(super) mod tasks;
pub(super) mod runs;
pub(super) mod grants;
pub(super) mod retirement;
pub(super) mod intents;
pub(super) mod reviewed;
mod dispatch_records;

/// The mutation's existing lock and caller admission, borrowed by each handler.
/// Timestamps stay at their original call sites rather than being cached here.
pub(super) struct MutationCtx<'a> {
    pub(super) service: &'a OrchestrationService,
    pub(super) locked: &'a LockedStore<'a>,
    pub(super) state: &'a mut OrchestrationState,
    pub(super) actor: &'a Actor,
    pub(super) caller: Option<usize>,
    pub(super) session_id: &'a str,
    pub(super) reviewed: Option<&'a Run>,
}

pub(super) enum Applied {
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
        OrchestrationAction::SupervisorStart { target, label } =>
            runs::supervisor_start(ctx, target, label),
        OrchestrationAction::RunAdopt { label } => runs::adopt(ctx, label),
        OrchestrationAction::RunBindSession { omp_session_id } =>
            runs::bind_session(ctx, omp_session_id),
        OrchestrationAction::RetirementNativeReceipt { retirement_id, outcome } =>
            retirement::native_receipt(ctx, retirement_id, outcome),
        OrchestrationAction::RunPropose {
            task_id, parent_run_id, label, target, prepare_brief, supersedes_run_id,
        } => runs::propose(ctx, task_id, parent_run_id, label, target,
            prepare_brief, supersedes_run_id),
        OrchestrationAction::GrantPrepare { run_id, plan_revision } =>
            grants::prepare(ctx, run_id, plan_revision),
        OrchestrationAction::GrantExecute { run_id, plan_revision, note } =>
            grants::execute(ctx, run_id, plan_revision, note),
        OrchestrationAction::Accept { run_id, expected_task_revision } =>
            intents::accept(ctx, run_id, expected_task_revision),
        OrchestrationAction::SendBack { run_id, text } =>
            grants::send_back(ctx, run_id, text),
        OrchestrationAction::CancelRun { run_id } => runs::cancel(ctx, run_id),
        OrchestrationAction::RetryLaunch { run_id } => runs::retry_launch(ctx, run_id),
        OrchestrationAction::ReconcileRun { run_id, recovery } =>
            runs::reconcile(ctx, run_id, recovery),
        OrchestrationAction::IntentResolve { intent_id, apply } =>
            intents::resolve(ctx, intent_id, apply),
        _ => Err(error("actor_forbidden", "Unsupported actor/action combination")),
    }
}
