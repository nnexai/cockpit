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
    task_root_authority(state, actor, caller, session_id, &root_id)?;
    bounded(&title, 256)?;
    bounded(&description, 16 * 1024)?;
    tasks_md::validate_authoring_creation(&title, &description)?;
    let task_id = tasks_md::validate_uuid(&task_id)?.to_string();
    let edges = normalize_task_edges(&depends_on)?;
    let follow = follow_up_of.as_deref().map(tasks_md::validate_uuid)
        .transpose()?.map(|id| id.to_string());
    if !edges.is_empty() || follow.is_some() {
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

pub(in crate::orchestration) fn scope_update(
    ctx: &mut MutationCtx<'_>,
    action: OrchestrationAction,
) -> Result<Applied, InspectionError> {
    let OrchestrationAction::TaskScopeUpdate {
        operation_id, root_id, task_id, run_id, expected_task_revision,
        expected_doc_revision, title, description,
    } = &action else { unreachable!() };
    // Durable identity is checked before revision or stage checks, including
    // replay after the worker has reported or the task has been accepted.
    let notify_worker = scope_actor(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id, task_id, run_id)?;
    let principal_key = super::resume::principal_key(ctx)?;
    if let Some(result) = super::resume::replay(ctx, operation_id,
        store::DurableOperationKind::TaskScopeUpdate, &action)? {
        let outcome = if ctx.state.scope_intents.iter().any(|intent| intent.operation_id == *operation_id
            && intent.principal_key == principal_key && intent.state == IntentState::Pending) {
            scope_intents::resume(ctx.locked, ctx.state, operation_id, &principal_key, true)
        } else { Ok(result) };
        ctx.service.revision.send_replace(ctx.state.revision);
        return Ok(Applied::Published(OrchestrationMutationResponse { revision: ctx.state.revision, result: outcome? }));
    }
    active_task_root(ctx.state, ctx.session_id, root_id)?;
    if title.is_none() && description.is_none() {
        return Err(error("invalid_task", "A scope update requires title or description"));
    }
    if let Some(title) = title { bounded(title, 256)?; }
    if let Some(description) = description { bounded(description, 16 * 1024)?; }
    let locked = ctx.locked;
    let document = locked.tasks(root_id)?;
    let task = mutable_task(ctx.state, root_id, task_id, &document)?;
    let run = &ctx.state.runs[scoped_index(ctx.state, ctx.session_id, run_id)?];
    if run.stage != RunStage::Working
        || projection::current_task_run(ctx.state, root_id, task_id).is_none_or(|current| current.run_id != *run_id) {
        return Err(error("attempt_stale", "Scope updates require the same current Working worker"));
    }
    dependencies::DependencyGraph::new(&document.tasks).require(task)?;
    let conflict = task.task_revision != *expected_task_revision || document.doc_revision != *expected_doc_revision;
    let result = OrchestrationActionResult::TaskScopeUpdated { update: TaskScopeUpdateResult {
        operation_id: operation_id.clone(),
        state: if conflict { TaskScopeUpdateState::Conflict } else { TaskScopeUpdateState::Pending },
        task_revision: Some(task.task_revision.clone()), doc_revision: Some(document.doc_revision.clone()),
        instruction_message_id: None, duplicate: false,
    }};
    let prepared = if conflict { None } else {
        Some(document.prepare_update(task_id, expected_task_revision, title.as_deref(), description.as_deref())?)
    };
    let intended_task_revision = prepared.as_ref().map_or_else(|| task.task_revision.clone(),
        |prepared| prepared.task.task_revision.clone());
    let intended_doc_revision = prepared.as_ref().map_or_else(|| document.doc_revision.clone(),
        |prepared| prepared.doc_revision.clone());
    let from = actor_ref(ctx.actor, ctx.caller, ctx.state);
    super::resume::record(ctx, operation_id.clone(), store::DurableOperationKind::TaskScopeUpdate,
        &action, root_id.clone(), result.clone(), Vec::new())?;
    ctx.state.scope_intents.push(scope_intents::ScopeIntent {
        operation_id: operation_id.clone(), root_id: root_id.clone(), task_id: task_id.clone(),
        principal_key: principal_key.clone(),
        run_id: run_id.clone(), expected_task_revision: expected_task_revision.clone(),
        expected_doc_revision: expected_doc_revision.clone(), intended_task_revision, intended_doc_revision,
        title: title.clone(), description: description.clone(), from, notify_worker,
        state: if conflict { IntentState::Conflict } else { IntentState::Pending },
    });
    let outcome = (|| {
        ctx.locked.save(ctx.state)?;
        if let Some(prepared) = prepared {
            match prepared.publish() {
                Ok(_) => {}
                Err(failure) if failure.code == "task_revision_conflict" => {}
                Err(failure) => return Err(failure),
            }
            scope_intents::resume(ctx.locked, ctx.state, operation_id, &principal_key, false)
        } else { Ok(result) }
    })();
    ctx.service.revision.send_replace(ctx.state.revision);
    Ok(Applied::Published(OrchestrationMutationResponse { revision: ctx.state.revision, result: outcome? }))
}

fn scope_actor(
    state: &OrchestrationState, actor: &Actor, caller: Option<usize>, session: &str,
    root_id: &str, task_id: &str, run_id: &str,
) -> Result<bool, InspectionError> {
    let root = &state.runs[scoped_index(state, session, root_id)?];
    let worker = &state.runs[scoped_index(state, session, run_id)?];
    if root.root_id != root.run_id || root.parent_run_id.is_some()
        || !matches!(root.kind, RunKind::Supervisor | RunKind::Adopted)
        || worker.kind != RunKind::Worker || worker.root_id != root_id
        || worker.task_id.as_deref() != Some(task_id) {
        return Err(error("actor_forbidden", "Scope update must identify one root and its task worker"));
    }
    match actor {
        Actor::Operator(_) => Ok(true),
        Actor::Agent(agent) => {
            let job = authenticated_task_caller(state, agent, caller)?;
            let run = &state.runs[job.run_index];
            if job.role == JobRole::Root && run.run_id == root_id { return Ok(true) }
            if run.run_id != run_id || !matches!(job.role, JobRole::Worker | JobRole::Child) {
                return Err(error("actor_forbidden", "Only this worker, its root or operator may refine scope"));
            }
            require_live_child(state, &job, run)?;
            Ok(false)
        }
        Actor::Bootstrap(_) => Err(error("actor_forbidden", "Physical bootstrap cannot refine canonical scope")),
    }
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
    let edges = dependencies::validate_dependencies(&document.tasks, &task.task_id, &depends_on)?;
    if state.runs.iter().any(|run| run.root_id == root_id
        && run.task_id.as_deref() == Some(task.task_id.as_str()) && run.stage != RunStage::Closed)
        && edges.iter().filter(|edge| !task.depends_on.contains(edge)).any(|edge|
            document.task(edge).ok().is_none_or(|prerequisite| !prerequisite.checked)) {
        return Err(error("task_relationships_live", "An active writer cannot acquire a newly unmet prerequisite"));
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
    task_root_authority(state, actor, caller, session_id, &root_id)?;
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

/// Ordinary task authority is durable job authority, not a native incarnation.
/// Credential validation also fences the worker's current checkout claim.
fn authenticated_task_caller(
    state: &OrchestrationState, agent: &AgentCaller, caller: Option<usize>,
) -> Result<AuthorizedJob, InspectionError> {
    let job = credentials::validate_job_auth(state, &agent.auth)?;
    let claimed_child = agent.agent_kind == Some(AgentKind::Subagent) || agent.subagent_id.is_some()
        || agent.provenance.as_ref().is_some_and(|provenance| provenance.child_id.is_some());
    if claimed_child && job.role != JobRole::Child {
        return Err(error("actor_forbidden", "A child must use its narrowed credential"));
    }
    if job.run_index != required_caller(caller)? {
        return Err(error("actor_forbidden", "Caller does not match its authenticated job"));
    }
    let run = &state.runs[job.run_index];
    if !(run.kind == RunKind::Adopted && run.dispatch.is_none()
        || run.dispatch.as_ref().is_some_and(|dispatch| dispatch.agent_started))
        || dispatch::recovery_incarnation_revoked(run) {
        return Err(error("actor_forbidden", "The authenticated job has not started or was revoked"));
    }
    Ok(job)
}

pub(super) fn task_root_authority(
    state: &OrchestrationState, actor: &Actor, caller: Option<usize>, session: &str, root_id: &str,
) -> Result<(), InspectionError> {
    let root = active_task_root(state, session, root_id)?;
    if matches!(actor, Actor::Operator(_)) { return Ok(()) }
    let Actor::Agent(agent) = actor else {
        return Err(error("actor_forbidden", "Physical bootstrap cannot author canonical tasks"));
    };
    let job = authenticated_task_caller(state, agent, caller)?;
    if state.runs[job.run_index].run_id != root.run_id || job.role != JobRole::Root {
        return Err(error("actor_forbidden", "Only the authenticated root or operator may author canonical tasks"));
    }
    Ok(())
}

pub(super) fn mutable_task<'a>(
    state: &OrchestrationState, root_id: &str, task_id: &str, document: &'a tasks_md::TaskDocument<'_>,
) -> Result<&'a Task, InspectionError> {
    let task = document.task(task_id)?;
    if task.checked || state.runs.iter().any(|run| run.root_id == root_id
        && run.task_id.as_deref() == Some(task.task_id.as_str()) && run.close_reason == Some(CloseReason::Accepted)) {
        return Err(error("task_checked", "Accepted canonical tasks are read-only"));
    }
    if state.task_intents.iter().any(|intent| intent.root_id == root_id && intent.task_id == task.task_id) {
        return Err(error("intent_conflict", "Resolve the existing acceptance intent first"));
    }
    if state.scope_intents.iter().any(|intent| intent.root_id == root_id && intent.task_id == task.task_id
        && intent.state == IntentState::Pending) {
        return Err(error("intent_conflict", "The existing scope update must settle first"));
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
    if matches!(actor, Actor::Operator(_)) {
        if current.is_some_and(|run| run.stage == RunStage::Working) {
            return Err(error("actor_forbidden", "Use TaskScopeUpdate to instruct the current Working worker"));
        }
        return Ok(());
    }
    let Actor::Agent(agent) = actor else {
        return Err(error("actor_forbidden", "Physical bootstrap cannot edit canonical tasks"));
    };
    let job = authenticated_task_caller(state, agent, caller)?;
    let run = &state.runs[job.run_index];
    if run.run_id == root_id {
        if job.role != JobRole::Root {
            return Err(error("actor_forbidden", "Only the authenticated root may coordinate canonical tasks"));
        }
        if current.is_some_and(|run| run.stage == RunStage::Working) {
            return Err(error("actor_forbidden", "Use TaskScopeUpdate to instruct the current Working worker"));
        }
        return Ok(());
    }
    if run.root_id != root_id || !matches!(job.role, JobRole::Worker | JobRole::Child) {
        return Err(error("actor_forbidden", "Caller does not own this canonical root"));
    }
    if run.kind != RunKind::Worker || run.stage != RunStage::Working
        || run.task_id.as_deref() != Some(task.task_id.as_str())
        || current.is_none_or(|current| current.run_id != run.run_id)
    {
        return Err(error("actor_forbidden", "Only the current Working worker may edit its own canonical task"));
    }
    require_live_child(state, &job, run)?;
    dependencies::DependencyGraph::new(&document.tasks).require(task)
}

fn require_live_child(state: &OrchestrationState, job: &AuthorizedJob, run: &Run) -> Result<(), InspectionError> {
    if job.role == JobRole::Child {
        let subject = job.subject_child_id.as_deref().ok_or_else(|| {
            error("actor_forbidden", "A narrowed child credential subject is required")
        })?;
        if !state.subagents.iter().any(|child| child.run_id == run.run_id
            && child.subagent_id == subject && child.status == SubagentStatus::Running) {
            return Err(error("actor_forbidden", "An authenticated live child subject is required"));
        }
    }
    Ok(())
}

pub(super) fn task_transition_target(
    state: &OrchestrationState, run: &Run, document: &tasks_md::TaskDocument<'_>,
) -> Result<(), InspectionError> {
    active_task_root(state, &run.session_id, &run.root_id)?;
    let task_id = run.task_id.as_deref().ok_or_else(|| error("task_not_found", "Worker task is missing"))?;
    let task = mutable_task(state, &run.root_id, task_id, document)?;
    let current = projection::current_task_run(state, &run.root_id, &task.task_id);
    if current.is_none_or(|current| current.run_id != run.run_id) {
        return Err(error("attempt_stale", "Worker no longer owns the current canonical task attempt"));
    }
    dependencies::DependencyGraph::new(&document.tasks).require(task)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture {
        directory: PathBuf,
        store: OrchestrationStore,
        state: OrchestrationState,
        task: Task,
        sibling: Task,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!("cockpit-task-job-auth-{}", id()));
            let store = OrchestrationStore::open(&directory).unwrap();
            let root_id = id();
            let mut root = new_run(&root_id, "task-job-auth", RunKind::Adopted,
                "Root".into(), &root_id, None, None, 1);
            root.stage = RunStage::Active;
            let locked = store.lock().unwrap();
            let task = locked.tasks(&root_id).unwrap().create_authoring_with_id(
                &id(), "Own task", "Own canonical content", &[], None, None, None,
            ).unwrap();
            let sibling = locked.tasks(&root_id).unwrap().create_authoring_with_id(
                &id(), "Sibling task", "Other canonical content", &[], None, None, None,
            ).unwrap();
            drop(locked);
            let mut worker = new_run(&id(), "task-job-auth", RunKind::Worker,
                "Worker".into(), &root_id, Some(root_id.clone()), Some(task.task_id.clone()), 1);
            worker.stage = RunStage::Working;
            let mut launch = dispatch(DispatchStep::Launched);
            launch.agent_started = true;
            worker.dispatch = Some(launch);
            let attachment = worker.attachment.as_ref().unwrap();
            let holder = claims::AttachmentReference {
                run_id: worker.run_id.clone(), attachment_id: attachment.attachment_id.clone(),
                attachment_epoch: attachment.epoch,
            };
            let checkout = CheckoutIdentity {
                key: "task-checkout".into(), machine_id: "task-machine".into(),
                path: directory.join("checkout").to_string_lossy().into_owned(),
                directory_identity: DirectoryIdentity { device: "1".into(), inode: "2".into() },
                git: None,
            };
            let mut state = OrchestrationState::default();
            state.runs.extend([root, worker]);
            claims::reserve_claim(&mut state, &checkout, &holder, &id()).unwrap();
            Self { directory, store, state, task, sibling }
        }

        fn actor(&mut self, index: usize, role: JobRole, subject: Option<&str>) -> Actor {
            let (record, path) = credentials::stage_credential(
                self.store.base(), &self.state.runs[index], role, subject,
            ).unwrap();
            self.state.credentials.push(record);
            Actor::Agent(AgentCaller {
                auth: read_job_auth(&path).unwrap(), provenance: None,
                endpoint_identity: String::new(), session_id: String::new(),
                workspace_id: String::new(), tab_id: String::new(), pane_id: String::new(),
                boot_id: None, terminal_id: None, native_session_id: None, env_run: None,
                omp_session_id: None, agent_kind: None, actual_agent_kind: None,
                subagent_id: None, main_omp_session_id: None, process: None,
            })
        }

        fn content(&self, actor: &Actor, index: usize, task: &Task) -> Result<(), InspectionError> {
            let locked = self.store.lock().unwrap();
            let document = locked.tasks(&self.state.runs[0].run_id).unwrap();
            task_content_target(&self.state, actor, Some(index), "task-job-auth",
                &self.state.runs[0].run_id, &task.task_id, &document)
        }

        fn scope_action(&self) -> OrchestrationAction {
            let locked = self.store.lock().unwrap();
            let document = locked.tasks(&self.state.runs[0].run_id).unwrap();
            OrchestrationAction::TaskScopeUpdate {
                operation_id: id(), root_id: self.state.runs[0].run_id.clone(),
                task_id: self.task.task_id.clone(), run_id: self.state.runs[1].run_id.clone(),
                expected_task_revision: document.task(&self.task.task_id).unwrap().task_revision.clone(),
                expected_doc_revision: document.doc_revision,
                title: Some("Updated scope".into()), description: Some("Additional user follow-up".into()),
            }
        }

        fn scope(&mut self, actor: &Actor, caller: Option<usize>, action: OrchestrationAction)
            -> Result<TaskScopeUpdateResult, InspectionError> {
            let service = OrchestrationService {
                store: OrchestrationStore::open(&self.directory).unwrap(),
                revision: watch::channel(self.state.revision).0,
            };
            let locked = self.store.lock().unwrap();
            locked.read().unwrap();
            let mut ctx = MutationCtx { service: &service, locked: &locked, state: &mut self.state,
                actor, caller, session_id: "task-job-auth", resolved: None };
            let Applied::Published(response) = scope_update(&mut ctx, action)? else { panic!("scope saves its own journal") };
            let OrchestrationActionResult::TaskScopeUpdated { update } = response.result else { panic!("scope receipt") };
            Ok(update)
        }

        fn dependencies(&mut self, edges: Vec<String>) -> Result<Task, InspectionError> {
            let service = OrchestrationService {
                store: OrchestrationStore::open(&self.directory).unwrap(),
                revision: watch::channel(self.state.revision).0,
            };
            let operator = Actor::Operator(OperatorOrigin::Browser);
            let locked = self.store.lock().unwrap();
            let root_id = self.state.runs[0].run_id.clone();
            let document = locked.tasks(&root_id).unwrap();
            let revision = document.task(&self.task.task_id).unwrap().task_revision.clone();
            let task_id = self.task.task_id.clone();
            let mut ctx = MutationCtx { service: &service, locked: &locked, state: &mut self.state,
                actor: &operator, caller: None, session_id: "task-job-auth", resolved: None };
            let Applied::Document(OrchestrationActionResult::Task { task }) =
                dependencies_set(&mut ctx, root_id, task_id, revision, document.doc_revision, edges)?
                else { panic!("task dependency receipt") };
            Ok(task)
        }

        fn journal_scope(&mut self, publish_markdown: bool) -> String {
            let action = self.scope_action();
            let OrchestrationAction::TaskScopeUpdate {
                operation_id, root_id, task_id, run_id, expected_task_revision,
                expected_doc_revision, title, description,
            } = &action else { unreachable!() };
            let locked = self.store.lock().unwrap();
            locked.read().unwrap();
            let document = locked.tasks(root_id).unwrap();
            let prepared = document.prepare_update(task_id, expected_task_revision,
                title.as_deref(), description.as_deref()).unwrap();
            let timestamp = now();
            self.state.operations.push(store::DurableOperationRecord {
                operation_id: operation_id.clone(), actor_run_id: None, root_id: root_id.clone(),
                principal_key: "operator".into(),
                run_id: None, resume_operation_id: None, kind: store::DurableOperationKind::TaskScopeUpdate,
                request_digest: super::super::resume::request_digest(&action, "task-job-auth").unwrap(),
                result: Some(OrchestrationActionResult::TaskScopeUpdated { update: TaskScopeUpdateResult {
                    operation_id: operation_id.clone(), state: TaskScopeUpdateState::Pending,
                    task_revision: Some(expected_task_revision.clone()), doc_revision: Some(expected_doc_revision.clone()),
                    instruction_message_id: None, duplicate: false,
                }}),
                effect_receipts: Vec::new(), created_at: timestamp.clone(), updated_at: timestamp,
            });
            self.state.scope_intents.push(scope_intents::ScopeIntent {
                operation_id: operation_id.clone(), root_id: root_id.clone(), task_id: task_id.clone(), run_id: run_id.clone(),
                principal_key: "operator".into(),
                expected_task_revision: expected_task_revision.clone(), expected_doc_revision: expected_doc_revision.clone(),
                intended_task_revision: prepared.task.task_revision.clone(), intended_doc_revision: prepared.doc_revision.clone(),
                title: title.clone(), description: description.clone(), from: ActorRef::Operator,
                notify_worker: true, state: IntentState::Pending,
            });
            locked.save(&mut self.state).unwrap();
            if publish_markdown { prepared.publish().unwrap(); }
            operation_id.clone()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn durable_task_authority_does_not_require_native_conversation_or_process() {
        let mut fixture = Fixture::new();
        let mut actor = fixture.actor(1, JobRole::Worker, None);
        fixture.content(&actor, 1, &fixture.task).unwrap();
        let Actor::Agent(caller) = &mut actor else { unreachable!() };
        caller.omp_session_id = Some("new-conversation".into());
        caller.main_omp_session_id = Some("other-conversation".into());
        caller.agent_kind = Some(AgentKind::Main);
        caller.process = Some(NativeProcessIdentity {
            pid: 99, start_ticks: 100, kernel_boot_id: Some("different-boot".into()),
        });
        fixture.content(&actor, 1, &fixture.task).unwrap();
        fixture.state.checkout_claims[0].state = claims::ClaimState::Conflict;
        assert!(fixture.content(&actor, 1, &fixture.task).is_err());
        fixture.state.checkout_claims[0].state = claims::ClaimState::Held;
        let Actor::Agent(caller) = &mut actor else { unreachable!() };
        caller.auth.attachment_epoch += 1;
        assert!(fixture.content(&actor, 1, &fixture.task).is_err());
    }

    #[test]
    fn inherited_main_credentials_and_physical_bootstrap_cannot_author_child_tasks() {
        let mut fixture = Fixture::new();
        let mut inherited = fixture.actor(1, JobRole::Worker, None);
        let Actor::Agent(caller) = &mut inherited else { unreachable!() };
        caller.agent_kind = Some(AgentKind::Subagent);
        assert!(fixture.content(&inherited, 1, &fixture.task).is_err());
        let Actor::Agent(caller) = &mut inherited else { unreachable!() };
        caller.agent_kind = Some(AgentKind::Main);
        caller.provenance = Some(CallerProvenance {
            agent_kind: Some(AgentKind::Main), child_id: Some("inherited-child".into()),
            main_session_id: None, native_session_id: None, process: None,
        });
        assert!(fixture.content(&inherited, 1, &fixture.task).is_err());
        let mut root = fixture.actor(0, JobRole::Root, None);
        let Actor::Agent(caller) = &mut root else { unreachable!() };
        caller.subagent_id = Some("inherited-root-child".into());
        assert!(task_root_authority(&fixture.state, &root, Some(0), "task-job-auth",
            &fixture.state.runs[0].run_id).is_err());
        let bootstrap = Actor::Bootstrap(BootstrapCaller {
            endpoint_identity: String::new(), session_id: String::new(),
            workspace_id: String::new(), tab_id: String::new(), pane_id: String::new(),
            boot_id: None, terminal_id: None, native_session_id: None,
            omp_session_id: None, agent_kind: None, actual_agent_kind: None, process: None,
        });
        assert!(fixture.content(&bootstrap, 1, &fixture.task).is_err());
        assert!(task_root_authority(&fixture.state, &bootstrap, None, "task-job-auth",
            &fixture.state.runs[0].run_id).is_err());
    }

    #[test]
    fn narrowed_child_subject_is_own_task_only_even_with_main_provenance() {
        let mut fixture = Fixture::new();
        let mut child = fixture.actor(1, JobRole::Child, Some("registered-child"));
        fixture.state.subagents.push(Subagent {
            run_id: fixture.state.runs[1].run_id.clone(), subagent_id: "other-child".into(),
            parent_subagent_id: None, bound_omp_session: None, role: None, label: "Child".into(),
            status: SubagentStatus::Running, summary: None, last_control: None, updated_at: now(),
        });
        assert!(fixture.content(&child, 1, &fixture.task).is_err());
        fixture.state.subagents[0].subagent_id = "registered-child".into();
        let Actor::Agent(caller) = &mut child else { unreachable!() };
        caller.agent_kind = Some(AgentKind::Main);
        caller.subagent_id = Some("other-child".into());
        fixture.content(&child, 1, &fixture.task).unwrap();
        assert!(fixture.content(&child, 1, &fixture.sibling).is_err());
        assert!(task_root_authority(&fixture.state, &child, Some(1), "task-job-auth",
            &fixture.state.runs[0].run_id).is_err());
        fixture.state.subagents[0].status = SubagentStatus::Done;
        assert!(fixture.content(&child, 1, &fixture.task).is_err());
    }

    #[test]
    fn task_root_authority_is_credential_role_and_root_scoped() {
        let mut fixture = Fixture::new();
        let root = fixture.actor(0, JobRole::Root, None);
        task_root_authority(&fixture.state, &root, Some(0), "task-job-auth",
            &fixture.state.runs[0].run_id).unwrap();
        let worker = fixture.actor(1, JobRole::Worker, None);
        assert!(task_root_authority(&fixture.state, &worker, Some(1), "task-job-auth",
            &fixture.state.runs[0].run_id).is_err());
        assert!(task_root_authority(&fixture.state, &root, Some(1), "task-job-auth",
            &fixture.state.runs[0].run_id).is_err());
        let other_id = id();
        let mut other = new_run(&other_id, "task-job-auth", RunKind::Adopted,
            "Other root".into(), &other_id, None, None, 1);
        other.stage = RunStage::Active;
        fixture.state.runs.push(other);
        assert!(task_root_authority(&fixture.state, &root, Some(0), "task-job-auth", &other_id).is_err());
        assert!(fixture.content(&root, 0, &fixture.task).is_err());
        fixture.state.runs[1].stage = RunStage::Reported;
        fixture.content(&root, 0, &fixture.task).unwrap();
    }

    #[test]
    fn job_credentials_do_not_bypass_current_worker_and_task_fences() {
        let mut fixture = Fixture::new();
        let worker = fixture.actor(1, JobRole::Worker, None);
        let original = fixture.state.runs[1].clone();
        // Neither an initialized report nor a matching permission plan is required.
        fixture.content(&worker, 1, &fixture.task).unwrap();
        for stage in [RunStage::Queued, RunStage::Starting, RunStage::Reported, RunStage::Closed] {
            fixture.state.runs[1] = original.clone();
            fixture.state.runs[1].stage = stage;
            assert!(fixture.content(&worker, 1, &fixture.task).is_err());
        }
        fixture.state.runs[1] = original.clone();
        let mut newer = original;
        newer.run_id = id();
        newer.attempt += 1;
        fixture.state.runs.push(newer);
        assert!(fixture.content(&worker, 1, &fixture.task).is_err());
        fixture.state.runs.pop();
        let locked = fixture.store.lock().unwrap();
        let mut document = locked.tasks(&fixture.state.runs[0].run_id).unwrap();
        let own = document.tasks.iter().position(|task| task.task_id == fixture.task.task_id).unwrap();
        document.tasks[own].checked = true;
        assert!(task_content_target(&fixture.state, &worker, Some(1), "task-job-auth",
            &fixture.state.runs[0].run_id, &fixture.task.task_id, &document).is_err());
        document.tasks[own].checked = false;
        document.tasks[own].depends_on.push(fixture.sibling.task_id.clone());
        assert!(task_content_target(&fixture.state, &worker, Some(1), "task-job-auth",
            &fixture.state.runs[0].run_id, &fixture.task.task_id, &document).is_err());
    }

    #[test]
    fn working_follow_up_writes_canonical_scope_and_instructs_same_worker_once() {
        let mut fixture = Fixture::new();
        let operator = Actor::Operator(OperatorOrigin::Browser);
        let action = fixture.scope_action();
        let applied = fixture.scope(&operator, None, action.clone()).unwrap();
        assert_eq!(applied.state, TaskScopeUpdateState::Applied);
        assert!(!applied.duplicate);
        assert_eq!(fixture.state.messages.len(), 1);
        let message = &fixture.state.messages[0];
        assert_eq!(message.to_run_id, fixture.state.runs[1].run_id);
        assert_eq!(message.kind, MessageKind::Instruction);
        assert_eq!(applied.instruction_message_id.as_deref(), Some(message.message_id.as_str()));
        assert!(fixture.state.scope_intents.is_empty());
        {
            let locked = fixture.store.lock().unwrap();
            let document = locked.tasks(&fixture.state.runs[0].run_id).unwrap();
            let task = document.task(&fixture.task.task_id).unwrap();
            assert_eq!(task.title, "Updated scope");
            assert_eq!(task.description, "Additional user follow-up");
            // Replay is the original receipt, even after another task edit.
            document.update(&fixture.sibling.task_id, &fixture.sibling.task_revision,
                Some("Unrelated user edit"), None).unwrap();
        }
        fixture.state.runs[1].stage = RunStage::Reported;
        let replay = fixture.scope(&operator, None, action.clone()).unwrap();
        assert!(replay.duplicate);
        assert_eq!(replay.task_revision, applied.task_revision);
        assert_eq!(fixture.state.messages.len(), 1);
        let mut changed = action;
        let OrchestrationAction::TaskScopeUpdate { title, .. } = &mut changed else { unreachable!() };
        *title = Some("Different request".into());
        assert_eq!(fixture.scope(&operator, None, changed).unwrap_err().code, "operation_conflict");
    }

    #[test]
    fn worker_self_refinement_has_no_instruction_and_replay_is_actor_bound() {
        let mut fixture = Fixture::new();
        let worker = fixture.actor(1, JobRole::Worker, None);
        let action = fixture.scope_action();
        let applied = fixture.scope(&worker, Some(1), action.clone()).unwrap();
        assert_eq!(applied.state, TaskScopeUpdateState::Applied);
        assert!(applied.instruction_message_id.is_none());
        assert!(fixture.state.messages.is_empty());
        assert!(fixture.scope(&worker, Some(1), action.clone()).unwrap().duplicate);
        let canonical_before = {
            let locked = fixture.store.lock().unwrap();
            let document = locked.tasks(&fixture.state.runs[0].run_id).unwrap();
            (document.doc_revision.clone(), serde_json::to_value(document.task(&fixture.task.task_id).unwrap()).unwrap())
        };
        let other_principal = fixture.scope(&Actor::Operator(OperatorOrigin::Native), None, action.clone()).unwrap();
        assert_eq!(other_principal.state, TaskScopeUpdateState::Conflict);
        assert!(!other_principal.duplicate);
        assert!(other_principal.instruction_message_id.is_none());
        assert!(fixture.state.messages.is_empty());
        let original_replay = fixture.scope(&worker, Some(1), action).unwrap();
        assert_eq!(original_replay.state, TaskScopeUpdateState::Applied);
        assert!(original_replay.duplicate);
        {
            let locked = fixture.store.lock().unwrap();
            let document = locked.tasks(&fixture.state.runs[0].run_id).unwrap();
            assert_eq!((document.doc_revision.clone(),
                serde_json::to_value(document.task(&fixture.task.task_id).unwrap()).unwrap()), canonical_before);
        }
    }

    #[test]
    fn stale_scope_cas_retains_original_draft_without_mail_or_canonical_overwrite() {
        let mut fixture = Fixture::new();
        let action = fixture.scope_action();
        {
            let locked = fixture.store.lock().unwrap();
            locked.tasks(&fixture.state.runs[0].run_id).unwrap().update(
                &fixture.task.task_id, &fixture.task.task_revision, Some("Fresh user scope"), None).unwrap();
        }
        let operator = Actor::Operator(OperatorOrigin::Browser);
        let conflict = fixture.scope(&operator, None, action.clone()).unwrap();
        assert_eq!(conflict.state, TaskScopeUpdateState::Conflict);
        assert!(fixture.state.messages.is_empty());
        assert_eq!(fixture.state.scope_intents[0].title.as_deref(), Some("Updated scope"));
        assert_eq!(fixture.state.scope_intents[0].description.as_deref(), Some("Additional user follow-up"));
        assert!(fixture.scope(&operator, None, action).unwrap().duplicate);
        let locked = fixture.store.lock().unwrap();
        assert_eq!(locked.tasks(&fixture.state.runs[0].run_id).unwrap()
            .task(&fixture.task.task_id).unwrap().title, "Fresh user scope");
    }

    #[test]
    fn scope_recovery_before_and_after_markdown_has_one_instruction() {
        for published in [false, true] {
            let mut fixture = Fixture::new();
            fixture.journal_scope(published);
            let locked = fixture.store.lock().unwrap();
            if published {
                // Changed unrelated content after the scope write survives recovery.
                locked.tasks(&fixture.state.runs[0].run_id).unwrap().update(
                    &fixture.sibling.task_id, &fixture.sibling.task_revision, Some("Unrelated fresh edit"), None).unwrap();
            }
            let mut durable = locked.read().unwrap();
            assert!(scope_intents::recover(&locked, &mut durable).unwrap());
            assert!(durable.scope_intents.is_empty());
            assert_eq!(durable.messages.len(), 1);
            assert_eq!(durable.messages[0].to_run_id, durable.runs[1].run_id);
            let mut reopened = locked.read().unwrap();
            assert!(!scope_intents::recover(&locked, &mut reopened).unwrap());
            assert_eq!(reopened.messages.len(), 1);
            let document = locked.tasks(&durable.runs[0].run_id).unwrap();
            assert_eq!(document.task(&fixture.task.task_id).unwrap().title, "Updated scope");
            if published {
                assert_eq!(document.task(&fixture.sibling.task_id).unwrap().title, "Unrelated fresh edit");
            }
        }
    }

    #[test]
    fn scope_recovery_never_overwrites_new_user_content_or_accepted_history() {
        for accepted in [false, true] {
            let mut fixture = Fixture::new();
            fixture.journal_scope(true);
            let locked = fixture.store.lock().unwrap();
            let document = locked.tasks(&fixture.state.runs[0].run_id).unwrap();
            let task = document.task(&fixture.task.task_id).unwrap();
            if accepted {
                document.check(&task.task_id, &task.task_revision, true).unwrap();
            } else {
                document.update(&task.task_id, &task.task_revision, Some("User edit after crash"), None).unwrap();
            }
            let mut durable = locked.read().unwrap();
            assert!(scope_intents::recover(&locked, &mut durable).unwrap());
            assert_eq!(durable.scope_intents[0].state, IntentState::Conflict);
            assert!(durable.messages.is_empty());
            assert!(!scope_intents::recover(&locked, &mut durable).unwrap());
            let document = locked.tasks(&durable.runs[0].run_id).unwrap();
            let task = document.task(&fixture.task.task_id).unwrap();
            assert_eq!(task.checked, accepted);
            if !accepted { assert_eq!(task.title, "User edit after crash"); }
        }
    }

    #[test]
    fn active_worker_dependencies_allow_met_additions_but_not_unmet_ones() {
        let mut fixture = Fixture::new();
        let prerequisite = fixture.sibling.task_id.clone();
        assert_eq!(fixture.dependencies(vec![prerequisite.clone()]).unwrap_err().code, "task_relationships_live");
        {
            let locked = fixture.store.lock().unwrap();
            locked.tasks(&fixture.state.runs[0].run_id).unwrap().check(
                &prerequisite, &fixture.sibling.task_revision, true).unwrap();
        }
        let task = fixture.dependencies(vec![prerequisite.clone()]).unwrap();
        assert_eq!(task.depends_on, vec![prerequisite]);
        assert!(fixture.dependencies(Vec::new()).unwrap().depends_on.is_empty());
    }

    #[test]
    fn accepted_scope_history_rejects_fresh_edits_and_other_worker_targets() {
        let mut fixture = Fixture::new();
        let operator = Actor::Operator(OperatorOrigin::Browser);
        let mut wrong_task = fixture.scope_action();
        let OrchestrationAction::TaskScopeUpdate { task_id, .. } = &mut wrong_task else { unreachable!() };
        *task_id = fixture.sibling.task_id.clone();
        assert_eq!(fixture.scope(&operator, None, wrong_task).unwrap_err().code, "actor_forbidden");
        {
            let locked = fixture.store.lock().unwrap();
            locked.tasks(&fixture.state.runs[0].run_id).unwrap().check(
                &fixture.task.task_id, &fixture.task.task_revision, true).unwrap();
        }
        let fresh = fixture.scope_action();
        assert_eq!(fixture.scope(&operator, None, fresh).unwrap_err().code, "task_checked");
        assert_eq!(fixture.dependencies(Vec::new()).unwrap_err().code, "task_checked");
        assert!(fixture.state.messages.is_empty());
    }

    #[test]
    fn crash_before_scope_write_keeps_new_document_edits_and_conflicts_the_draft() {
        let mut fixture = Fixture::new();
        fixture.journal_scope(false);
        let locked = fixture.store.lock().unwrap();
        locked.tasks(&fixture.state.runs[0].run_id).unwrap().update(
            &fixture.sibling.task_id, &fixture.sibling.task_revision, Some("New user document edit"), None).unwrap();
        let mut durable = locked.read().unwrap();
        assert!(scope_intents::recover(&locked, &mut durable).unwrap());
        assert_eq!(durable.scope_intents[0].state, IntentState::Conflict);
        assert_eq!(durable.scope_intents[0].description.as_deref(), Some("Additional user follow-up"));
        assert!(durable.messages.is_empty());
        let document = locked.tasks(&durable.runs[0].run_id).unwrap();
        assert_eq!(document.task(&fixture.task.task_id).unwrap().title, "Own task");
        assert_eq!(document.task(&fixture.sibling.task_id).unwrap().title, "New user document edit");
    }

    #[test]
    fn root_liaison_updates_scope_and_instructs_its_existing_worker_once() {
        let mut fixture = Fixture::new();
        let root = fixture.actor(0, JobRole::Root, None);
        let action = fixture.scope_action();
        let update = fixture.scope(&root, Some(0), action.clone()).unwrap();
        assert_eq!(update.state, TaskScopeUpdateState::Applied);
        assert_eq!(fixture.state.messages.len(), 1);
        assert!(matches!(&fixture.state.messages[0].from,
            ActorRef::Run { run_id } if *run_id == fixture.state.runs[0].run_id));
        assert_eq!(fixture.state.messages[0].to_run_id, fixture.state.runs[1].run_id);
        assert!(fixture.scope(&root, Some(0), action).unwrap().duplicate);
        assert_eq!(fixture.state.messages.len(), 1);
    }

    #[test]
    fn principal_local_scope_ids_do_not_resume_or_settle_another_principals_draft() {
        let mut fixture = Fixture::new();
        let root = fixture.actor(0, JobRole::Root, None);
        let operator = Actor::Operator(OperatorOrigin::Browser);
        let rejected = fixture.scope_action();
        {
            let locked = fixture.store.lock().unwrap();
            locked.tasks(&fixture.state.runs[0].run_id).unwrap().update(
                &fixture.task.task_id, &fixture.task.task_revision, Some("Fresh user edit"), None).unwrap();
        }
        assert_eq!(fixture.scope(&operator, None, rejected.clone()).unwrap().state, TaskScopeUpdateState::Conflict);
        let mut current = fixture.scope_action();
        let OrchestrationAction::TaskScopeUpdate { operation_id: rejected_id, .. } = &rejected else { unreachable!() };
        let OrchestrationAction::TaskScopeUpdate { operation_id, .. } = &mut current else { unreachable!() };
        *operation_id = rejected_id.clone();
        let applied = fixture.scope(&root, Some(0), current.clone()).unwrap();
        assert_eq!(applied.state, TaskScopeUpdateState::Applied);
        assert!(!applied.duplicate);
        assert_eq!(fixture.state.messages.len(), 1);
        assert_eq!(fixture.state.scope_intents.len(), 1);
        assert_eq!(fixture.state.scope_intents[0].principal_key, "operator");
        let operator_replay = fixture.scope(&operator, None, rejected).unwrap();
        assert_eq!(operator_replay.state, TaskScopeUpdateState::Conflict);
        assert!(operator_replay.duplicate);
        let root_replay = fixture.scope(&root, Some(0), current).unwrap();
        assert_eq!(root_replay.state, TaskScopeUpdateState::Applied);
        assert!(root_replay.duplicate);
        assert_eq!(fixture.state.messages.len(), 1);
    }
}
