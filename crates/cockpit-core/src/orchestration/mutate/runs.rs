use super::{Applied, MutationCtx};
use super::super::*;
use super::tasks::{mutable_task, task_root_authority};
use cockpit_protocol::projects::WorkspaceRecoveryAction;

pub(in crate::orchestration) fn supervisor_start(
    ctx: &mut MutationCtx<'_>,
    action: OrchestrationAction,
) -> Result<Applied, InspectionError> {
    use super::super::store::DurableOperationKind;
    let OrchestrationAction::SupervisorStart {
        operation_id, mode, target, label, selected_root_id, expected_project_key,
    } = &action else { unreachable!() };
    operator(ctx.actor)?;
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::SupervisorStart, &action)? {
        return Ok(Applied::Document(result));
    }
    let mut resolved_project = ctx.resolved.and_then(|r| r.project.clone());
    if target.is_some() && resolved_project.is_none() {
        return Err(error("project_identity_unavailable", "Start requires a host-resolved project identity"));
    }
    if expected_project_key.as_ref().is_some_and(|key| resolved_project.as_ref().is_none_or(|p| &p.key != key)) {
        return Err(error("project_identity_conflict", "The focused project changed before Start"));
    }
    if let Some(checkout) = ctx.resolved.and_then(|r| r.checkout.as_ref()) {
        project_identity::revalidate_checkout(checkout)?;
    }
    if let Some(checkout) = ctx.resolved.and_then(|r| r.reselected_checkout.as_ref()) {
        project_identity::revalidate_checkout(checkout)?;
        let index = selected_root_id.as_deref().map(|run_id| scoped_index(ctx.state, ctx.session_id, run_id)).transpose()?
            .ok_or_else(|| error("checkout_reselection_unavailable", "Re-selection requires an explicitly selected root"))?;
        let run = &ctx.state.runs[index];
        if *mode != SupervisorStartMode::ReuseOrResume || !root_checkout_reselection_pending(run)
            || !matches!(target, Some(DispatchTarget::ExistingSpace { .. })) {
            return Err(error("checkout_reselection_unavailable", "Only a claim-free root waiting on a changed checkout can be re-selected"));
        }
        if resolved_project.as_ref().map(|project| &project.key) != run.project.as_ref().map(|project| &project.key) {
            return Err(error("project_identity_conflict", "Re-selection must retain the root project"));
        }
        if let Some(DispatchTarget::ExistingSpace { workspace_id }) = target
            && resolved_project.as_ref().and_then(|project| project.source_workspace_id.as_ref()) != Some(workspace_id) {
            return Err(error("checkout_reselection_space_mismatch", "Re-selection must originate in the root's retained Space"));
        }
    }
    if *mode == SupervisorStartMode::ReuseOrResume {
        if let Some(workspace_id) = resolved_project.as_ref().and_then(|p| p.source_workspace_id.as_deref()) {
            let unresolved = ctx.state.runs.iter().any(|run| run.session_id == ctx.session_id
                && run.parent_run_id.is_none() && run.stage != RunStage::Closed
                && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted) && run.project.is_none()
                && (run.attachment.as_ref().and_then(|a| a.location.as_ref()).is_some_and(|l| l.workspace_id == workspace_id)
                    || run.setup.as_ref().and_then(|s| s.project_workspace_id.as_deref()) == Some(workspace_id)
                    || match run.target.as_ref() {
                        Some(DispatchTarget::ExistingSpace { workspace_id: source })
                        | Some(DispatchTarget::SpaceWorktree { workspace_id: source, .. }) => source == workspace_id,
                        _ => false,
                    }));
            if unresolved {
                return Err(error("project_identity_unavailable", "An associated open root requires owner project reconciliation before reuse"));
            }
        }
        if let Some(project) = &resolved_project {
            if let Some(index) = preferred_project_root(&ctx.state.runs, ctx.session_id, &project.key, selected_root_id.as_deref()) {
                let run = &ctx.state.runs[index];
                let root_id = run.run_id.clone();
                let observed_connected = has_current_binding(ctx.state, run)
                    && ctx.resolved.and_then(|r| r.attachment_observation.as_ref()).is_some_and(|observed|
                    run.attachment.as_ref().is_some_and(|current| observed.attachment_id == current.attachment_id
                        && observed.epoch == current.epoch && observed.process == current.process
                        && observed.shell == current.shell && observed.machine_id == current.machine_id
                        && observed.launch_mode == current.launch_mode
                        && observed.conversation == current.conversation
                        && observed.launch_attempt == current.launch_attempt
                        && project_identity::same_location(observed.location.as_ref(), current.location.as_ref())
                        && !matches!(current.state, AttachmentState::Quiescing | AttachmentState::Revoked)
                        && observed.process.is_some() && observed.state == AttachmentState::Connected))
                    && run.resume.as_ref().is_none_or(|resume|
                        matches!(resume.phase, ResumePhase::Connected | ResumePhase::Reattached));
                let (outcome, resume_operation_id) = if observed_connected {
                    if let Some(expected) = ctx.resolved.and_then(|input| input.root_attachment_expected.as_ref()) {
                        let current = ctx.state.runs[index].attachment.as_ref().ok_or_else(|| error("attachment_conflict", "Root attachment is missing"))?;
                        if !project_identity::same_root_attachment(current, expected) {
                            return Err(error("attachment_conflict", "Root attachment changed during Start observation"));
                        }
                        project_identity::revalidate_root_observation(ctx.state, &ctx.state.runs[index], expected)?;
                    }
                    ctx.state.runs[index].attachment = ctx.resolved.and_then(|r| r.attachment_observation.clone());
                    (SupervisorStartOutcome::Reused, None)
                } else {
                    let resume_id = id();
                    let epoch = ctx.state.runs[index].attachment.as_ref().map(|a| a.epoch);
                    super::resume::resume_from_start(ctx, OrchestrationAction::RunResume {
                        run_id: root_id.clone(), operation_id: resume_id.clone(),
                        expected_attachment_epoch: epoch,
                    })?;
                    let effective = ctx.state.runs[index].resume.as_ref().map(|r| r.operation_id.clone()).unwrap_or(resume_id);
                    (SupervisorStartOutcome::Resuming, Some(effective))
                };
                let result = OrchestrationActionResult::SupervisorStarted { start: SupervisorStartResult {
                    root_id: root_id.clone(), outcome, operation_id: operation_id.clone(), resume_operation_id, duplicate: false,
                }};
                super::resume::record(ctx, operation_id.clone(), DurableOperationKind::SupervisorStart, &action,
                    root_id, result.clone(), Vec::new())?;
                return Ok(Applied::Machine(result));
            }
        }
    }
    let run_id = id();
    let label = label.clone().unwrap_or_else(|| "Supervisor".into());
    bounded(&label, 256)?;
    let mut checkout = ctx.resolved.and_then(|r| r.checkout.clone());
    let target = match target.clone() {
        Some(target) => unfocused(target),
        None => {
            let path = prepare_project_root(&ctx.service.store.base().join("supervisors").join(&run_id))?;
            let (project, directory) = project_identity::resolve_owned_directory(&path)?;
            resolved_project = Some(project);
            checkout = Some(directory);
            DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
                path: path.to_string_lossy().into_owned(), label: Some(label.clone()), task_name: None, focus: false,
            }}
        }
    };
    let mut run = new_run(&run_id, ctx.session_id, RunKind::Supervisor, label, &run_id, None, None, 1);
    run.launch_config = ctx.service.launch_config.clone();
    run.target = Some(target);
    run.project = resolved_project;
    run.checkout = checkout;
    run.stage = RunStage::Starting;
    run.dispatch = Some(dispatch(DispatchStep::SetupPending));
    run.brief = supervisor_guidance().into();
    ctx.state.runs.push(run);
    let result = OrchestrationActionResult::SupervisorStarted { start: SupervisorStartResult {
        root_id: run_id.clone(), outcome: SupervisorStartOutcome::Created,
        operation_id: operation_id.clone(), resume_operation_id: None, duplicate: false,
    }};
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::SupervisorStart, &action,
        run_id, result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(in crate::orchestration) fn adopt(
    ctx: &mut MutationCtx<'_>, label: String, location: RunLocation,
) -> Result<Applied, InspectionError> {
    operator(ctx.actor)?;
    bounded(&label, 256)?;
    if location.session_id != ctx.session_id || location.terminal_id.is_none()
        || ctx.state.runs.iter().any(|run| run.attachment.as_ref().and_then(|a| a.location.as_ref())
            .is_some_and(|owned| owned.endpoint_identity == location.endpoint_identity
                && owned.terminal_id == location.terminal_id)) {
        return Err(error("attachment_conflict", "Adoption requires an exact unowned terminal"));
    }
    let observed = ctx.resolved.and_then(|r| r.attachment_observation.as_ref())
        .ok_or_else(|| error("attachment_observation_unavailable", "Adoption requires fresh exact physical placement"))?;
    if !project_identity::same_location(observed.location.as_ref(), Some(&location)) {
        return Err(error("attachment_conflict", "Selected adoption placement changed"));
    }
    let run_id = id();
    let mut run = new_run(&run_id, ctx.session_id, RunKind::Adopted, label, &run_id, None, None, 1);
    run.launch_config = ctx.service.launch_config.clone();
    run.stage = RunStage::Starting;
    run.project = ctx.resolved.and_then(|r| r.project.clone());
    run.checkout = ctx.resolved.and_then(|r| r.checkout.clone());
    let attachment = run.attachment.as_mut().expect("adopted attachment");
    attachment.location = Some(location);
    attachment.process = observed.process.clone();
    attachment.shell = observed.shell.clone();
    attachment.machine_id = observed.machine_id.clone();
    attachment.state = AttachmentState::Connecting;
    let (credential, _) = credentials::stage_credential(ctx.service.base(), &run, JobRole::Root)?;
    ctx.state.credentials.push(credential);
    ctx.state.runs.push(run);
    Ok(Applied::Machine(OrchestrationActionResult::Run { run_id, attempt: 1 }))
}

pub(in crate::orchestration) fn worker_start(
    ctx: &mut MutationCtx<'_>,
    action: OrchestrationAction,
) -> Result<Applied, InspectionError> {
    use super::super::store::DurableOperationKind;
    let OrchestrationAction::WorkerStart {
        operation_id, root_id, task_id, expected_task_revision, expected_doc_revision,
        target, label, brief, cleanup_policy,
    } = &action else { unreachable!() };
    root_authority(ctx, root_id)?;
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::WorkerStart, &action)? {
        return Ok(Applied::Document(result));
    }
    bounded(brief, 16 * 1024)?;
    let document = ctx.locked.tasks(root_id)?;
    let task = mutable_task(ctx.state, root_id, task_id, &document)?;
    if task.task_revision != *expected_task_revision || document.doc_revision != *expected_doc_revision {
        return Err(error("task_revision_conflict", "Canonical task or document changed before Start"));
    }
    if dependencies::DependencyGraph::new(&document.tasks).evaluate(task).state == TaskDependencyState::Invalid {
        return Err(error("task_dependencies_invalid", "Task relationships are structurally invalid"));
    }
    let generation = ownership_index(ctx.state, root_id, task_id);
    let generation = ctx.state.task_ownership[generation].generation.checked_add(1)
        .ok_or_else(|| error("invalid_counter", "Task assignment generation exhausted"))?;
    let start = create_attempt(ctx, AttemptRequest {
        operation_id, root_id, task_id, target: target.clone(), brief,
        cleanup_policy: *cleanup_policy, label: label.as_deref(), generation,
    })?;
    let result = OrchestrationActionResult::WorkerStarted { start };
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::WorkerStart, &action,
        root_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(in crate::orchestration) struct AttemptRequest<'a> {
    pub operation_id: &'a str,
    pub root_id: &'a str,
    pub task_id: &'a str,
    pub target: DispatchTarget,
    pub brief: &'a str,
    pub cleanup_policy: WorkerCleanupPolicy,
    pub label: Option<&'a str>,
    pub generation: u64,
}

fn ownership_index(state: &mut OrchestrationState, root_id: &str, task_id: &str) -> usize {
    if let Some(index) = state.task_ownership.iter().position(|record|
        record.root_id == root_id && record.task_id == task_id) { return index; }
    state.task_ownership.push(TaskOwnership { root_id: root_id.into(), task_id: task_id.into(),
        generation: 0, owner: None, schedule: None });
    state.task_ownership.len() - 1
}

pub(in crate::orchestration) fn create_attempt(
    ctx: &mut MutationCtx<'_>, req: AttemptRequest<'_>,
) -> Result<WorkerStartResult, InspectionError> {
    tasks_md::validate_uuid(req.operation_id)?;
    bounded(req.brief, 16 * 1024)?;
    safe_counter(req.generation)?;
    let document = ctx.locked.tasks(req.root_id)?;
    let task = mutable_task(ctx.state, req.root_id, req.task_id, &document)?;
    if dependencies::DependencyGraph::new(&document.tasks).evaluate(task).state == TaskDependencyState::Invalid {
        return Err(error("task_dependencies_invalid", "Task relationships are structurally invalid"));
    }
    if ctx.state.runs.iter().any(|run| run.root_id == req.root_id
        && run.task_id.as_deref() == Some(req.task_id) && run.stage != RunStage::Closed) {
        return Err(error("task_has_active_run", "Replace the current task attempt instead of authorizing a second writer"));
    }
    let ownership_index = ownership_index(ctx.state, req.root_id, req.task_id);
    let ownership = &ctx.state.task_ownership[ownership_index];
    if ownership.owner.is_some() || ownership.generation.checked_add(1) != Some(req.generation) {
        return Err(error("assignment_generation_conflict", "Task ownership changed before attempt creation"));
    }
    let attempt = next_attempt(ctx.state, req.root_id, req.task_id)?;
    let run_id = id();
    let label = req.label.unwrap_or(&task.title);
    bounded(label, 256)?;
    let mut run = new_run(&run_id, ctx.session_id, RunKind::Worker, label.into(), req.root_id,
        Some(req.root_id.into()), Some(req.task_id.into()), attempt);
    run.launch_config = ctx.service.launch_config.clone();
    run.assignment_generation = Some(req.generation);
    run.target = Some(unfocused(req.target));
    run.brief = req.brief.into();
    run.task_revision_at_start = Some(task.task_revision.clone());
    run.cleanup_policy = Some(req.cleanup_policy);
    run.stage = RunStage::Queued;
    run.dispatch = Some(dispatch(DispatchStep::Planning));
    run.project = ctx.resolved.and_then(|resolved| resolved.project.clone())
        .or_else(|| ctx.state.runs.iter().find(|root| root.run_id == req.root_id).and_then(|root| root.project.clone()));
    if let Some(DispatchTarget::OwnedAttemptDirectory { source_root, .. }) = &run.target {
        if run.project.as_ref().is_none_or(|project|
            !matches!(&project.source, ProjectSource::Directory { root } if root == source_root)) {
            return Err(error("attempt_source_changed", "Owned attempt must preserve its original Directory project"));
        }
        let private_parent = ctx.service.base().join("attempts");
        let source = std::path::Path::new(source_root);
        if !source.is_absolute() || private_parent.starts_with(source) || source.starts_with(&private_parent) {
            return Err(error("attempt_source_overlap", "Private attempt directories must be outside the borrowed source"));
        }
    }
    ctx.state.runs.push(run);
    Ok(WorkerStartResult { operation_id: req.operation_id.into(), run_id, attempt, duplicate: false })
}

pub(in crate::orchestration) fn next_attempt(state: &OrchestrationState, root_id: &str, task_id: &str) -> Result<u32, InspectionError> {
    state.runs.iter().filter(|run| run.root_id == root_id && run.task_id.as_deref() == Some(task_id))
        .map(|run| run.attempt).max().unwrap_or(0).checked_add(1)
        .ok_or_else(|| error("orchestration_state_full", "Task attempt exhausted"))
}

fn root_authority(ctx: &MutationCtx<'_>, root_id: &str) -> Result<(), InspectionError> {
    task_root_authority(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id)?;
    if let Actor::Agent(agent) = ctx.actor {
        if credentials::validate_caller(ctx.state, agent)?.role != JobRole::Root {
            return Err(error("actor_forbidden", "Only the owning liaison or explicit operator may authorize task execution"));
        }
    }
    Ok(())
}

fn execution_provenance(ctx: &MutationCtx<'_>, root_id: &str) -> DecisionProvenance {
    match ctx.actor {
        Actor::Operator(origin) => DecisionProvenance {
            origin: match origin { OperatorOrigin::Browser => DecisionOrigin::Browser, OperatorOrigin::Native => DecisionOrigin::Native },
            supervisor_run_id: None, conversation: None,
        },
        Actor::Agent(caller) => DecisionProvenance { origin: DecisionOrigin::Supervisor,
            supervisor_run_id: Some(root_id.into()), conversation: caller.conversation.clone() },
    }
}

pub(super) fn schedule(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    use super::super::store::DurableOperationKind;
    let (operation_id, root_id, task_id, kind) = match &action {
        OrchestrationAction::TaskSchedule { operation_id, root_id, task_id, .. } =>
            (operation_id, root_id, task_id, DurableOperationKind::TaskSchedule),
        OrchestrationAction::TaskScheduleCancel { operation_id, root_id, task_id } =>
            (operation_id, root_id, task_id, DurableOperationKind::TaskScheduleCancel),
        _ => unreachable!(),
    };
    root_authority(ctx, root_id)?;
    if let Some(result) = super::resume::replay(ctx, operation_id, kind, &action)? {
        return Ok(Applied::Document(result));
    }
    let document = ctx.locked.tasks(root_id)?;
    let task = document.task(task_id)?;
    if task.checked { return Err(error("task_accepted", "Accepted tasks cannot be scheduled")); }
    let mut corrected_attempt = false;
    let schedule = if let OrchestrationAction::TaskSchedule {
        target, brief, cleanup_policy, label, expected_task_revision, expected_doc_revision, ..
    } = &action {
        bounded(brief, 16 * 1024)?;
        if let Some(label) = label { bounded(label, 256)?; }
        if expected_task_revision.as_ref().is_some_and(|revision| revision != &task.task_revision)
            || expected_doc_revision.as_ref().is_some_and(|revision| revision != &document.doc_revision) {
            return Err(error("task_revision_conflict", "Canonical task or document changed before scheduling"));
        }
        if dependencies::DependencyGraph::new(&document.tasks).evaluate(task).state == TaskDependencyState::Invalid {
            return Err(error("task_dependencies_invalid", "Task relationships are structurally invalid"));
        }
        if target.is_some() {
            let mut attempts = ctx.state.runs.iter().enumerate().filter(|(_, run)|
                run.root_id == *root_id && run.task_id.as_deref() == Some(task_id.as_str())
                    && run.stage != RunStage::Closed);
            let failed = attempts.next().filter(|(_, run)| run.session_id == ctx.session_id
                && run.parent_run_id.as_deref() == Some(root_id.as_str())
                && super::super::dispatch::plan_failed_before_effects(run))
                .map(|(index, run)| (index, run.run_id.clone(), run.assignment_generation));
            if let Some((run_index, run_id, Some(generation))) = failed && attempts.next().is_none() {
                let unclaimed = ctx.state.task_ownership.iter().find(|record|
                    record.root_id == *root_id && record.task_id == *task_id)
                    .is_some_and(|record| record.owner.is_none() && record.generation.checked_add(1) == Some(generation));
                let physical_receipt = ctx.state.checkout_claims.iter().any(|claim|
                    claim.holder.as_ref().is_some_and(|holder| holder.run_id == run_id))
                    || ctx.state.owned_resources.iter().any(|resource| resource.run_id == run_id
                        || resource.consumers.iter().any(|consumer| consumer.run_id == run_id))
                    || ctx.state.resource_requests.iter().any(|request| request.holder.run_id == run_id)
                    || ctx.state.operations.iter().any(|operation|
                        (operation.run_id.as_deref() == Some(run_id.as_str())
                            || operation.actor_run_id.as_deref() == Some(run_id.as_str())
                            || matches!(&operation.result, Some(OrchestrationActionResult::WorkerStarted { start })
                                | Some(OrchestrationActionResult::WorkerReplaced { start, .. }) if start.run_id == run_id))
                            && !operation.effect_receipts.is_empty());
                if !unclaimed || physical_receipt {
                    return Err(error("plan_failed_attempt_unproven",
                        "Failed planning attempt has ownership or physical receipts; preserve it and inspect run show before choosing worker-replace"));
                }
                super::ownership::supersede_current(ctx, root_id, task_id, generation, Some(&run_id))?;
                let provenance = execution_provenance(ctx, root_id);
                let by = actor_ref(ctx.actor, ctx.caller, ctx.state);
                ctx.state.runs[run_index].annotations.push(decision_annotation(
                    by, &provenance,
                    "Superseded unstarted plan-failed attempt by explicit re-schedule; no physical effects were recorded"));
                corrected_attempt = true;
            }
        }
        Some(TaskSchedule { operation_id: operation_id.clone(), principal_key: super::resume::principal_key(ctx)?,
            target: target.clone().map(unfocused), brief: brief.clone(), cleanup_policy: *cleanup_policy,
            label: label.clone(), scheduled_at: now() })
    } else { None };
    let index = ownership_index(ctx.state, root_id, task_id);
    if schedule.is_some() && ctx.state.task_ownership[index].schedule.is_some() && !corrected_attempt {
        return Err(error("task_already_scheduled", "An existing schedule may only be corrected with an explicit target after proven pre-effect planning failure; preserve started or uncertain attempts and use worker-replace"));
    }
    if schedule.is_some() && !corrected_attempt && ctx.state.runs.iter().any(|run|
        run.root_id == *root_id && run.task_id.as_deref() == Some(task_id.as_str())
            && run.stage != RunStage::Closed
            && run.dispatch.as_ref().is_some_and(|dispatch| dispatch.step == DispatchStep::PlanFailed)) {
        return Err(error("plan_failed_attempt_unproven",
            "Re-schedule requires one never-started failed planning attempt and an explicit target; inspect run show and preserve uncertain receipts"));
    }
    ctx.state.task_ownership[index].schedule = schedule;
    let result = OrchestrationActionResult::TaskScheduled { operation_id: operation_id.clone(), task_id: task_id.clone(), duplicate: false };
    super::resume::record(ctx, operation_id.clone(), kind, &action, root_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(super) fn worker_replace(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    use super::super::store::DurableOperationKind;
    let OrchestrationAction::WorkerReplace { operation_id, root_id, task_id,
        expected_run_id, expected_assignment_generation, reason } = &action else { unreachable!() };
    root_authority(ctx, root_id)?;
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::WorkerReplace, &action)? {
        return Ok(Applied::Document(result));
    }
    bounded(reason, 4096)?;
    let old_index = run_index(ctx.state, expected_run_id)?;
    let old = &ctx.state.runs[old_index];
    if old.root_id != *root_id || old.task_id.as_ref() != Some(task_id) {
        return Err(error("assignment_generation_conflict", "Expected attempt does not belong to this task"));
    }
    let resolved = ctx.resolved.ok_or_else(|| error("replacement_placement_unresolved", "Replacement needs a fresh physical placement observation"))?;
    let (target, placement) = if resolved.writer_exit_proof.is_some() {
        (old.target.clone().ok_or_else(|| error("replacement_placement_unresolved", "Old attempt has no retained target"))?,
            Some(ReplacementPlacement::SameCheckout))
    } else {
        if old.checkout.as_ref().is_some_and(|checkout| resolved.checkout.as_ref() != Some(checkout))
            || old.project.as_ref().is_some_and(|project| resolved.project.as_ref() != Some(project)) {
            return Err(error("replacement_source_changed", "Old checkout or original project identity changed before replacement"));
        }
        match resolved.replacement_source.as_ref() {
            Some(ReplacementSource::GitHead { commit, source_workspace_id }) => {
                if old.project.as_ref().is_none_or(|project| !matches!(project.source, ProjectSource::Git { .. })) {
                    return Err(error("replacement_source_changed", "Git replacement requires the unchanged original Git project"));
                }
                if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(error("replacement_source_unavailable", "Git replacement needs a verified committed HEAD"));
                }
                let source = old.project.as_ref().and_then(|project| project.source_workspace_id.as_ref())
                    .or_else(|| ctx.state.runs.iter().find(|run| run.run_id == *root_id)
                        .and_then(|run| run.project.as_ref()).and_then(|project| project.source_workspace_id.as_ref()));
                if source_workspace_id.as_ref().zip(source).is_some_and(|(actual, expected)| actual != expected) {
                    return Err(error("replacement_source_changed", "Git replacement source Space changed"));
                }
                let target = if let Some(workspace_id) = source_workspace_id {
                    validate_identity(workspace_id)?;
                    DispatchTarget::SpaceWorktree { workspace_id: workspace_id.clone(), branch: None, base_ref: Some(commit.clone()) }
                } else if let Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Create { repository_id,
                    label, task_name, artifact_url, linked_artifact_urls, .. } }) = &old.target {
                    DispatchTarget::Setup { request: WorkspaceSetupRequest::Create { repository_id: repository_id.clone(),
                        branch: None, base_ref: Some(commit.clone()), checkout_path: None, label: label.clone(),
                        task_name: task_name.clone(), artifact_url: artifact_url.clone(), linked_artifact_urls: linked_artifact_urls.clone(), focus: false } }
                } else {
                    return Err(error("replacement_source_unavailable", "No source Space or original explicit repository target authorizes this replacement"));
                };
                (target, Some(ReplacementPlacement::IsolatedWorktree { base_commit: commit.clone() }))
            }
            Some(ReplacementSource::Directory { root, source_workspace_id }) => {
                if old.project.as_ref().is_none_or(|project|
                    !matches!(&project.source, ProjectSource::Directory { root: original } if original == root)) {
                    return Err(error("replacement_source_changed", "Directory replacement requires the unchanged original Directory project"));
                }
                let private_parent = ctx.service.base().join("attempts");
                let original = std::path::Path::new(root);
                if !original.is_absolute() || private_parent.starts_with(original) || original.starts_with(&private_parent) {
                    return Err(error("replacement_source_overlap", "Private attempt directories must be outside the borrowed source"));
                }
                (DispatchTarget::OwnedAttemptDirectory { source_root: root.clone(), source_workspace_id: source_workspace_id.clone() }, None)
            }
            None => return Err(error("replacement_source_unavailable", "Replacement requires a revalidated Git checkpoint or original Directory source")),
        }
    };
    // Preserve the original task brief rather than nesting every replacement's
    // recovery header. Progress is evidence to inspect, never a stale directive.
    let original_brief = ctx.state.task_ownership.iter().find(|ownership|
            ownership.root_id == *root_id && ownership.task_id == *task_id)
        .and_then(|ownership| ownership.schedule.as_ref()).map(|schedule| schedule.brief.as_str())
        .or_else(|| ctx.state.runs.iter().filter(|run|
            run.root_id == *root_id && run.task_id.as_deref() == Some(task_id))
            .min_by_key(|run| run.attempt).map(|run| run.brief.as_str()))
        .unwrap_or(old.brief.as_str());
    let old_checkout = old.checkout.as_ref().map(|checkout| checkout.path.as_str())
        .unwrap_or("not yet physically observed");
    let base = match &placement {
        Some(ReplacementPlacement::IsolatedWorktree { base_commit }) => base_commit.as_str(),
        Some(ReplacementPlacement::SameCheckout) => "retained checkout; inspect its current HEAD",
        Some(ReplacementPlacement::IsolatedDirectory { .. }) | None => "Directory source; no Git checkpoint",
    };
    let brief = format!(
        "Replacement of run {} (attempt {}). Reason: {}\nOld checkout: {}\nNew attempt base: {}\n\
         Read the canonical task and previous run's reports/progress before proceeding. Previous progress is advisory; \
         do not replay stale instructions. The old checkout and uncommitted work are retained.\n\n{}",
        old.run_id, old.attempt, reason, old_checkout, base, original_brief,
    );
    bounded(&brief, 16 * 1024)?;
    let cleanup_policy = old.cleanup_policy.unwrap_or(WorkerCleanupPolicy::OwnedWhenSafe);
    let label = old.label.clone();
    let retained_checkout = if resolved.writer_exit_proof.is_some() {
        let setup = old.setup.clone().filter(|setup| setup.workspace_id.is_some())
            .ok_or_else(|| error("replacement_placement_unresolved", "Same-checkout replacement requires the exact retained Space receipt"))?;
        let claim_id = old.checkout_claim_id.clone()
            .ok_or_else(|| error("writer_exit_unproven", "Old writer has no protected checkout"))?;
        Some((setup, old.project.clone(), claim_id))
    } else { None };
    super::ownership::supersede_current(ctx, root_id, task_id, *expected_assignment_generation, Some(expected_run_id))?;
    let ownership = ctx.state.task_ownership.iter().find(|record| record.root_id == *root_id && record.task_id == *task_id).expect("superseded ownership");
    let generation = ownership.generation.checked_add(1).ok_or_else(|| error("invalid_counter", "Task generation exhausted"))?;
    let start = create_attempt(ctx, AttemptRequest { operation_id, root_id, task_id, target, brief: &brief,
        cleanup_policy, label: Some(&label), generation })?;
    let successor_index = run_index(ctx.state, &start.run_id)?;
    let placement = match placement {
        Some(placement) => placement,
        None => ReplacementPlacement::IsolatedDirectory { directory: ctx.service.base().join("attempts").join(&start.run_id)
            .to_str().ok_or_else(|| error("replacement_source_unavailable", "Private attempt path is not UTF-8"))?.into() },
    };
    if let (Some(proof), Some((setup, project, claim_id))) = (&resolved.writer_exit_proof, retained_checkout) {
        claims::transfer_claim_to_successor(ctx.state, &claim_id, proof, &start.run_id, operation_id)?;
        // The owned checkout is reused only with exact physical exit proof.
        ctx.state.runs[successor_index].setup = Some(setup);
        ctx.state.runs[successor_index].project = project;
        ctx.state.runs[successor_index].stage = RunStage::Starting;
        ctx.state.runs[successor_index].dispatch = Some(dispatch(DispatchStep::SetupPending));
    }
    let authority = ReplacementRetirementAuthority { operation_id: operation_id.clone(), previous_run_id: expected_run_id.clone(),
        previous_generation: *expected_assignment_generation, replacement_run_id: start.run_id.clone() };
    ctx.state.runs[old_index].retirement = retirement::retirement_for_replacement(&ctx.state.runs[old_index], authority, &now());
    let provenance = execution_provenance(ctx, root_id);
    let by = actor_ref(ctx.actor, ctx.caller, ctx.state);
    ctx.state.runs[old_index].annotations.push(decision_annotation(by, &provenance, reason));
    let result = OrchestrationActionResult::WorkerReplaced { start, previous_run_id: expected_run_id.clone(), assignment_generation: generation, placement };
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::WorkerReplace, &action, root_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(in crate::orchestration) fn cancel(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
) -> Result<Applied, InspectionError> {
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    if dispatch::recovery_close_pending(&state.runs[index]) {
        return Err(error("recovery_pending", "Owned launch cancellation is still being observed; tracking cannot be closed until its bounded effect settles"));
    }
    if state.runs[index].stage == RunStage::Closed {
        return Err(error("invalid_stage", "Run is already closed"));
    }
    state.runs[index].stage = RunStage::Closed;
    state.runs[index].close_reason = Some(CloseReason::Cancelled);
    let from = actor_ref(actor, caller, state);
    messages::append(state, messages::AppendMessage { from: from, to_run_id: &run_id, message_id: &format!("cancel-{}", id()), kind: MessageKind::CancelRequest, text: "Tracking is closed for this run. Please stop and report outstanding effects. This advisory request does not guarantee process termination; resources and descendants are retained.", in_reply_to: None, report: None, stale: false, escalated_from: None })?;
    let annotation = decision_annotation(
        actor_ref(actor, caller, state),
        &provenance,
        "Tracking closed; advisory cancellation recorded; descendants retained",
    );
    state.runs[index].annotations.push(annotation);
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}


pub(in crate::orchestration) fn reconcile(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
    recovery: Option<WorkspaceRecoveryAction>,
) -> Result<Applied, InspectionError> {
    let (index, provenance) = management_target(ctx.state, ctx.actor, ctx.caller, ctx.session_id, &run_id)?;
    if dispatch::recovery_close_pending(&ctx.state.runs[index]) {
        return Err(error("recovery_pending", "Owned launch cancellation is still being observed"));
    }
    if ctx.state.runs[index].stage == RunStage::Closed {
        return Err(error("invalid_stage", "Closed runs cannot be reconciled"));
    }
    let by = actor_ref(ctx.actor, ctx.caller, ctx.state);
    let run = &mut ctx.state.runs[index];
    let d = run.dispatch.as_mut().ok_or_else(|| error("invalid_stage", "No setup to reconcile"))?;
    if d.step != DispatchStep::SetupUnknown
        && !(d.step == DispatchStep::NeedsReview && d.launch_tag.is_none()
            && run.setup.as_ref().is_some_and(|setup| setup.operation_id.is_some())) {
        return Err(error("invalid_stage", "Only uncertain setup accepts environment reconciliation; use Resume for attachment recovery"));
    }
    d.step = DispatchStep::SetupUnknown;
    d.recovery = Some(recovery.ok_or_else(|| error("invalid_stage", "Choose an explicit setup recovery action"))?);
    d.error = None;
    d.updated_at = now();
    run.annotations.push(decision_annotation(by, &provenance,
        "Setup reconciliation requested; environment recovery requires fresh checkout proof"));
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

fn unfocused(mut target: DispatchTarget) -> DispatchTarget {
    if let DispatchTarget::Setup { request } = &mut target {
        match request {
            WorkspaceSetupRequest::Create { focus, .. }
            | WorkspaceSetupRequest::Open { focus, .. } => *focus = false,
        }
    }
    target
}

fn schedule_principal_current(state: &OrchestrationState, root: &Run, schedule: &TaskSchedule) -> bool {
    let operator_schedule = schedule.principal_key == "operator";
    root.stage == RunStage::Active && (operator_schedule || has_current_binding(state, root))
        && state.operations.iter().any(|receipt| receipt.operation_id == schedule.operation_id
            && receipt.kind == super::super::store::DurableOperationKind::TaskSchedule
            && receipt.root_id == root.run_id
            && (if operator_schedule { receipt.actor_run_id.is_none() } else { receipt.actor_run_id.as_ref() == Some(&root.run_id) })
            && receipt.principal_key == schedule.principal_key
            && matches!(&receipt.result, Some(OrchestrationActionResult::TaskScheduled { .. })))
}

fn schedule_ready(state: &OrchestrationState, root: &Run, task: &Task,
    graph: &dependencies::DependencyGraph<'_>, max_parallel: u32) -> bool {
    !task.checked && matches!(graph.evaluate(task).state, TaskDependencyState::None | TaskDependencyState::Satisfied)
        && !state.runs.iter().any(|run| run.root_id == root.run_id
            && run.task_id.as_deref() == Some(task.task_id.as_str()) && run.stage != RunStage::Closed)
        && state.runs.iter().filter(|run| run.root_id == root.run_id && run.kind == RunKind::Worker
            && run.stage != RunStage::Closed).count() < max_parallel as usize
}

impl OrchestrationService {
    /// Read the durable execution queue once; no model turn or physical effect.
    pub fn schedule_candidates(&self, max_parallel: u32) -> Result<Vec<ScheduleCandidate>, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let mut candidates = Vec::new();
        for root in state.runs.iter().filter(|run| run.run_id == run.root_id && run.stage == RunStage::Active) {
            let document = locked.tasks(&root.run_id)?;
            let graph = dependencies::DependencyGraph::new(&document.tasks);
            for ownership in state.task_ownership.iter().filter(|record| record.root_id == root.run_id) {
                let Some(schedule) = &ownership.schedule else { continue };
                let Ok(task) = document.task(&ownership.task_id) else { continue };
                if !schedule_principal_current(&state, root, schedule)
                    || !schedule_ready(&state, root, task, &graph, max_parallel.min(root.launch_config.max_parallel)) { continue; }
                candidates.push(ScheduleCandidate { session_id: root.session_id.clone(), root_id: root.run_id.clone(),
                    task_id: task.task_id.clone(), schedule: schedule.clone(), next_attempt: next_attempt(&state, &root.run_id, &task.task_id)? });
            }
        }
        Ok(candidates)
    }

    /// Recheck source, authorization, dependency and parallel gates under the
    /// same durable lock that publishes the next attempt.
    pub fn scheduled_start(&self, candidate: &ScheduleCandidate, resolved: &ResolvedMutationInput) -> Result<Option<WorkerStartResult>, InspectionError> {
        use super::super::store::DurableOperationKind;
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let root_index = scoped_index(&state, &candidate.session_id, &candidate.root_id)?;
        let root = &state.runs[root_index];
        let Some(ownership_index) = state.task_ownership.iter().position(|record|
            record.root_id == candidate.root_id && record.task_id == candidate.task_id) else { return Ok(None); };
        let Some(schedule) = state.task_ownership[ownership_index].schedule.clone() else { return Ok(None); };
        if schedule.operation_id != candidate.schedule.operation_id || schedule.principal_key != candidate.schedule.principal_key
            || !schedule_principal_current(&state, root, &schedule) { return Ok(None); }
        let namespace = Uuid::parse_str(&schedule.operation_id)
            .map_err(|_| error("orchestration_state_invalid", "Stored schedule operation UUID is invalid"))?;
        let operation_id = Uuid::new_v5(&namespace, format!("attempt-{}", candidate.next_attempt).as_bytes()).to_string();
        if let Some(record) = state.operations.iter().find(|record| record.operation_id == operation_id
            && record.kind == DurableOperationKind::WorkerStart && record.principal_key == schedule.principal_key) {
            if let Some(OrchestrationActionResult::WorkerStarted { start }) = &record.result {
                let mut start = start.clone();
                start.duplicate = true;
                return Ok(Some(start));
            }
        }
        let document = locked.tasks(&candidate.root_id)?;
        let task = document.task(&candidate.task_id)?;
        let graph = dependencies::DependencyGraph::new(&document.tasks);
        if !schedule_ready(&state, root, task, &graph, root.launch_config.max_parallel)
            || next_attempt(&state, &candidate.root_id, &candidate.task_id)? != candidate.next_attempt { return Ok(None); }
        let mut target = match (resolved.normalized_target.clone(), schedule.target.clone()) {
            (Some(target @ DispatchTarget::SpaceWorktree { .. }), Some(DispatchTarget::Setup {
                request: cockpit_protocol::projects::WorkspaceSetupRequest::Open { path, .. },
            })) if matches!(&candidate.schedule.target, Some(DispatchTarget::Setup {
                request: cockpit_protocol::projects::WorkspaceSetupRequest::Open { path: observed, .. },
            }) if observed == &path)
                && resolved.checkout.is_none() && resolved.project.as_ref().is_some_and(|project|
                matches!(&project.source, ProjectSource::Git { .. }) && project.repository_id.is_some()
                    && matches!(&target, DispatchTarget::SpaceWorktree { workspace_id, branch: None, base_ref: None }
                        if project.source_workspace_id.as_ref() == Some(workspace_id))) => target,
            (Some(_), _) => return Err(error("schedule_source_unresolved",
                "Only a verified Git source Space may normalize an authorized path into an isolated worktree")),
            (None, Some(target)) => target,
            (None, None) => {
                let project = root.project.as_ref()
                    .ok_or_else(|| error("schedule_source_unresolved", "Default placement requires the liaison's original project"))?;
                let source_workspace_id = project.source_workspace_id.clone()
                    .ok_or_else(|| error("schedule_source_unresolved", "Default placement requires the liaison's actual project Space"))?;
                match &project.source {
                    ProjectSource::Git { .. } => DispatchTarget::SpaceWorktree { workspace_id: source_workspace_id, branch: None, base_ref: None },
                    ProjectSource::Directory { root } => DispatchTarget::OwnedAttemptDirectory {
                        source_root: root.clone(), source_workspace_id: Some(source_workspace_id),
                    },
                }
            },
        };
        let project = resolved.project.as_ref().ok_or_else(|| error("schedule_source_unresolved", "Scheduled attempt requires a freshly resolved source"))?;
        if schedule.target.is_none() && root.project.as_ref().is_some_and(|expected| expected.key != project.key) {
            return Err(error("project_conflict", "Scheduled source no longer belongs to the liaison project"));
        }
        let inputs = input_snapshot(&state, &locked, &candidate.session_id, &candidate.root_id, &candidate.task_id)?;
        if let Some(commit) = materialized_inputs(&inputs, project, resolved.schedule_inputs.as_ref())? {
            freeze_input_base(&mut target, &commit);
        }
        let action = OrchestrationAction::WorkerStart { operation_id: operation_id.clone(), root_id: candidate.root_id.clone(),
            task_id: candidate.task_id.clone(), expected_task_revision: task.task_revision.clone(), expected_doc_revision: document.doc_revision.clone(),
            target: target.clone(), label: schedule.label.clone(), brief: schedule.brief.clone(), cleanup_policy: schedule.cleanup_policy };
        let generation = state.task_ownership[ownership_index].generation.checked_add(1)
            .ok_or_else(|| error("invalid_counter", "Task generation exhausted"))?;
        let actor = Actor::Operator(OperatorOrigin::Native);
        let mut ctx = MutationCtx { service: self, locked: &locked, state: &mut state, actor: &actor,
            caller: None, session_id: &candidate.session_id, resolved: Some(resolved) };
        let start = create_attempt(&mut ctx, AttemptRequest { operation_id: &operation_id,
            root_id: &candidate.root_id, task_id: &candidate.task_id, target, brief: &schedule.brief,
            cleanup_policy: schedule.cleanup_policy, label: schedule.label.as_deref(), generation })?;
        super::resume::record(&mut ctx, operation_id.clone(), DurableOperationKind::WorkerStart, &action,
            candidate.root_id.clone(), OrchestrationActionResult::WorkerStarted { start: start.clone() }, Vec::new())?;
        let receipt = ctx.state.operations.last_mut().expect("recorded scheduled attempt");
        receipt.principal_key = schedule.principal_key;
        messages::append(ctx.state, messages::AppendMessage { from: ActorRef::Dispatcher, to_run_id: &candidate.root_id,
            message_id: &format!("scheduled-start-{operation_id}"), kind: MessageKind::Observation,
            text: "Scheduled task dependencies are satisfied; its authorized attempt is queued.",
            in_reply_to: None, report: None, stale: false, escalated_from: None })?;
        let revision = locked.save(ctx.state)?;
        self.revision.send_replace(revision);
        Ok(Some(start))
    }
}

pub(in crate::orchestration) fn input_snapshot(state: &OrchestrationState, locked: &store::LockedStore<'_>,
    session_id: &str, root_id: &str, task_id: &str) -> Result<ScheduleInputSnapshot, InspectionError> {
    let root = &state.runs[scoped_index(state, session_id, root_id)?];
    if root.run_id != root.root_id || root.parent_run_id.is_some() {
        return Err(error("task_root_invalid", "Dependency input inspection requires the canonical root"));
    }
    let document = locked.tasks(root_id)?;
    let task = document.task(task_id)?;
    let mut accepted_sources = Vec::with_capacity(task.depends_on.len());
    for dependency in &task.depends_on {
        let Some(run) = dependency_inputs::accepted_dependency_run(state, &document, session_id, root_id, dependency)? else { continue; };
        let source = run.source_output.as_ref().expect("accepted source validated");
        let (message_id, revision) = super::integration::output_identity(source);
        accepted_sources.push(ScheduleInputSource { task_id: dependency.clone(), run_id: run.run_id.clone(),
            result_message_id: message_id.into(), task_revision: revision.into(),
            project_key: run.project.as_ref().map(|project| project.key.clone()), source_output: source.clone(),
            integration: Some(run.integration.clone()) });
    }
    Ok(ScheduleInputSnapshot { root_id: root_id.into(), task_id: task_id.into(),
        doc_revision: document.doc_revision.clone(), task_revision: task.task_revision.clone(), accepted_sources })
}

pub(in crate::orchestration) fn materialized_inputs(snapshot: &ScheduleInputSnapshot, project: &ProjectIdentity,
    facts: Option<&ScheduleInputFacts>) -> Result<Option<String>, InspectionError> {
    if facts.is_some_and(|facts| &facts.snapshot != snapshot) {
        return Err(error("dependency_input_changed", "Accepted dependency source or canonical document changed during input verification"));
    }
    let mut requires_base = false;
    for source in &snapshot.accepted_sources {
        let SourceOutput::Code { repository_key, .. } = &source.source_output else { continue; };
        if repository_key != &project.key { continue; }
        requires_base = true;
        if facts.is_none_or(|facts| !facts.verified_code_runs.contains(&source.run_id)) {
            return Err(error("dependency_input_unavailable", "Same-repository accepted code is not proven present in this attempt's base"));
        }
    }
    if !requires_base { return Ok(facts.and_then(|facts| facts.base_commit.clone())); }
    let commit = facts.and_then(|facts| facts.base_commit.as_ref()).filter(|commit|
        matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| error("dependency_input_unavailable", "Dependency input verification needs an immutable observed commit"))?;
    Ok(Some(commit.clone()))
}

fn freeze_input_base(target: &mut DispatchTarget, commit: &str) {
    match target {
        DispatchTarget::SpaceWorktree { base_ref, .. } => *base_ref = Some(commit.into()),
        DispatchTarget::Setup { request: WorkspaceSetupRequest::Create { base_ref, .. } } => *base_ref = Some(commit.into()),
        // Explicit mutable checkouts are re-verified and pinned in SetupSummary
        // during launch reservation, rather than silently retargeted.
        DispatchTarget::ExistingSpace { .. } | DispatchTarget::Setup { request: WorkspaceSetupRequest::Open { .. } }
            | DispatchTarget::OwnedAttemptDirectory { .. } => {},
    }
}

impl OrchestrationService {
    pub fn schedule_input_requirements(&self, candidate: &ScheduleCandidate) -> Result<ScheduleInputSnapshot, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        input_snapshot(&state, &locked, &candidate.session_id, &candidate.root_id, &candidate.task_id)
    }

    pub(crate) fn launch_input_requirements(&self, run: &Run) -> Result<Option<ScheduleInputSnapshot>, InspectionError> {
        let Some(task_id) = &run.task_id else { return Ok(None); };
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let current = &state.runs[scoped_index(&state, &run.session_id, &run.run_id)?];
        if current.assignment_generation != run.assignment_generation || current.stage == RunStage::Closed {
            return Err(error("assignment_generation_conflict", "Attempt changed before dependency input observation"));
        }
        input_snapshot(&state, &locked, &run.session_id, &run.root_id, task_id).map(Some)
    }

    pub(crate) fn verify_launch_inputs(&self, expected: &Run, project: &ProjectIdentity,
        facts: Option<&ScheduleInputFacts>) -> Result<Option<String>, InspectionError> {
        let Some(task_id) = &expected.task_id else { return Ok(None); };
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let current = &state.runs[scoped_index(&state, &expected.session_id, &expected.run_id)?];
        if current.assignment_generation != expected.assignment_generation || current.stage == RunStage::Closed {
            return Err(error("assignment_generation_conflict", "Attempt changed before dependency input verification"));
        }
        let document = locked.tasks(&expected.root_id)?;
        let task = document.task(task_id)?;
        dependencies::DependencyGraph::new(&document.tasks).require(task)?;
        materialized_inputs(&input_snapshot(&state, &locked, &expected.session_id, &expected.root_id, task_id)?, project, facts)
    }
}
