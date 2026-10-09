use super::{Applied, MutationCtx};
use super::super::*;
use super::tasks::{active_task_root, task_transition_target};
use super::super::retirement;

pub(in crate::orchestration) fn accept(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
    expected_task_revision: String,
) -> Result<Applied, InspectionError> {
    let service = ctx.service;
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    require_stage(&state.runs[index], RunStage::Reported)?;
    if state.runs[index].result.as_ref().and_then(|r| r.outcome)
        != Some(ReportOutcome::Succeeded)
    {
        return Err(error(
            "invalid_stage",
            "Only an explicit successful result can be accepted",
        ));
    }
    if state
        .task_intents
        .iter()
        .any(|intent| intent.run_id == run_id)
    {
        return Err(error(
            "intent_conflict",
            "Resolve the existing acceptance intent first",
        ));
    }
    let root_id = state.runs[index].root_id.clone();
    let task_id = state.runs[index]
        .task_id
        .clone()
        .ok_or_else(|| error("task_not_found", "Root runs do not own tasks"))?;
    let document = locked.tasks(&root_id)?;
    let task = document.task(&task_id)?;
    if task.task_revision != expected_task_revision {
        return Err(error(
            "task_revision_conflict",
            "Task changed before acceptance",
        ));
    }
    task_transition_target(state, &state.runs[index], &document, false)?;
    let intent_id = id();
    state.task_intents.push(TaskIntent {
        intent_id: intent_id.clone(),
        root_id: root_id.clone(),
        task_id: task_id.clone(),
        run_id: run_id.clone(),
        expected_task_revision: expected_task_revision.clone(),
        state: IntentState::Pending,
        origin: Some(provenance.origin),
        supervisor_run_id: provenance.supervisor_run_id.clone(),
        omp_session_id: provenance.omp_session_id.clone(),
        result_message_id: state.runs[index]
            .result
            .as_ref()
            .map(|r| r.message_id.clone()),
    });
    let annotation = Annotation {
        at: now(),
        by: actor_ref(actor, caller, state),
        text: format!(
            "Acceptance requested for Result {} at task revision {}; origin {:?}, main session {:?}.",
            state.runs[index]
                .result
                .as_ref()
                .expect("successful result")
                .message_id,
            expected_task_revision,
            provenance.origin,
            provenance.omp_session_id
        ),
    };
    state.runs[index].annotations.push(annotation);
    let revision = locked.save(state)?;
    service.revision.send_replace(revision);
    match document.check(&task_id, &expected_task_revision, true) {
        Ok(_) => {
            state.task_intents.retain(|i| i.intent_id != intent_id);
            retirement::close_accepted(state, index, RetirementTrigger::Accept, &expected_task_revision);
            let annotation = Annotation {
                at: now(),
                by: actor_ref(actor, caller, state),
                text: format!(
                    "Accepted Result at exact task revision {expected_task_revision}."
                ),
            };
            state.runs[index].annotations.push(annotation);
        }
        Err(failure) => {
            if failure.code == "task_revision_conflict" {
                state
                    .task_intents
                    .iter_mut()
                    .find(|i| i.intent_id == intent_id)
                    .expect("intent exists")
                    .state = IntentState::Conflict;
                let revision = locked.save(state)?;
                service.revision.send_replace(revision);
            }
            return Err(failure);
        }
    }
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

pub(in crate::orchestration) fn resolve(
    ctx: &mut MutationCtx<'_>,
    intent_id: String,
    apply: bool,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    operator(actor)?;
    let intent = state
        .task_intents
        .iter()
        .find(|i| i.intent_id == intent_id)
        .cloned()
        .ok_or_else(|| error("intent_not_found", "Acceptance intent does not exist"))?;
    let index = scoped_index(state, session_id, &intent.run_id)?;
    if apply {
        require_stage(&state.runs[index], RunStage::Reported)?;
        if state.runs[index]
            .result
            .as_ref()
            .and_then(|report| report.outcome)
            != Some(ReportOutcome::Succeeded)
            || intent.result_message_id.as_ref().is_none_or(|message_id| {
                state.runs[index]
                    .result
                    .as_ref()
                    .is_none_or(|report| &report.message_id != message_id)
            })
        {
            return Err(error(
                "intent_conflict",
                "Acceptance requires its unchanged explicit successful Result",
            ));
        }
        let document = locked.tasks(&intent.root_id)?;
        active_task_root(state, session_id, &intent.root_id)?;
        if state.runs[index].root_id != intent.root_id
            || state.runs[index].task_id.as_deref() != Some(intent.task_id.as_str())
            || projection::current_task_run(state, &intent.root_id, &intent.task_id)
                .is_none_or(|run| run.run_id != intent.run_id) {
            return Err(error("intent_conflict", "Acceptance intent no longer owns this canonical task"));
        }
        dependencies::DependencyGraph::new(&document.tasks).require(document.task(&intent.task_id)?)?;
        let revision = document.task(&intent.task_id)?.task_revision.clone();
        document.check(&intent.task_id, &revision, true)?;
        retirement::close_accepted(state, index, RetirementTrigger::OperatorConflictResolution, &revision);
        state.runs[index].annotations.push(Annotation {
            at: now(), by: ActorRef::Operator,
            text: format!("Operator resolved acceptance conflict using current canonical task revision {revision}; prior requested Result {:?}.", intent.result_message_id),
        });
    }
    state.task_intents.retain(|i| i.intent_id != intent_id);
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

impl OrchestrationService {
    pub(crate) fn recover_intents(&self) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let assignment_recovery = assignments::recover(&locked, &mut state);
        // Publish committed recovery (including before an error) or externally
        // changed state, but do not wake the dispatcher for an unchanged revision.
        self.revision.send_if_modified(|revision| {
            if *revision == state.revision {
                return false;
            }
            *revision = state.revision;
            true
        });
        assignment_recovery?;
        let mut changed = false;
        for intent in state.task_intents.clone() {
            if intent.state == IntentState::Conflict {
                continue;
            }
            let index = run_index(&state, &intent.run_id)?;
            let run = &state.runs[index];
            let valid_receipt = run.root_id == intent.root_id
                && run.task_id.as_ref() == Some(&intent.task_id)
                && run.result.as_ref().and_then(|report| report.outcome)
                    == Some(ReportOutcome::Succeeded)
                && (run.stage == RunStage::Reported
                    || (run.stage == RunStage::Closed
                        && run.close_reason == Some(CloseReason::Accepted)));
            if !valid_receipt
                || intent.result_message_id.as_ref().is_none_or(|message_id| {
                    state.runs[index]
                        .result
                        .as_ref()
                        .is_none_or(|report| &report.message_id != message_id)
                })
            {
                state
                    .task_intents
                    .iter_mut()
                    .find(|entry| entry.intent_id == intent.intent_id)
                    .expect("intent exists")
                    .state = IntentState::Conflict;
                changed = true;
                continue;
            }
            let mut dependency_conflict = None;
            let outcome = match locked.tasks(&intent.root_id) {
                Ok(document) => match document.task(&intent.task_id) {
                    Ok(task) if task.checked => {
                        task.task_revision == intent.expected_task_revision
                            || document.checked_matches_revision(
                                &intent.task_id,
                                &intent.expected_task_revision,
                            )?
                    }
                    Ok(task) if task.task_revision == intent.expected_task_revision => {
                        match dependencies::DependencyGraph::new(&document.tasks).require(task) {
                            Err(failure) => {
                                dependency_conflict = Some(format!("{}: {}", failure.code, failure.message));
                                false
                            }
                            Ok(()) => match document.check(&intent.task_id, &intent.expected_task_revision, true) {
                                Ok(_) => true,
                                Err(failure) if failure.code == "task_revision_conflict" => false,
                                Err(failure) => return Err(failure),
                            },
                        }
                    }
                    Ok(_) => false,
                    Err(failure)
                        if matches!(
                            failure.code.as_str(),
                            "task_not_found" | "task_id_duplicate"
                        ) =>
                    {
                        false
                    }
                    Err(failure) => return Err(failure),
                },
                Err(failure) => return Err(failure),
            };
            if outcome {
                let index = run_index(&state, &intent.run_id)?;
                retirement::close_accepted(&mut state, index, RetirementTrigger::AcceptRecovery, &intent.expected_task_revision);
                state
                    .task_intents
                    .retain(|i| i.intent_id != intent.intent_id);
                state.runs[index].annotations.push(Annotation {
                    at: now(),
                    by: intent.supervisor_run_id.as_ref().map_or(ActorRef::Operator, |run_id| ActorRef::Run { run_id: run_id.clone() }),
                    text: format!("Recovered acceptance of Result {:?} at exact task revision {}; origin {:?}, main session {:?}.",
                        intent.result_message_id, intent.expected_task_revision, intent.origin, intent.omp_session_id),
                });
            } else {
                state
                    .task_intents
                    .iter_mut()
                    .find(|i| i.intent_id == intent.intent_id)
                    .expect("intent exists")
                    .state = IntentState::Conflict;
                if let Some(mut reason) = dependency_conflict {
                    if reason.len() > 2048 {
                        let mut end = 2048;
                        while !reason.is_char_boundary(end) { end -= 1; }
                        reason.truncate(end);
                    }
                    state.runs[index].annotations.push(Annotation {
                        at: now(), by: ActorRef::Operator,
                        text: format!("Acceptance recovery retained conflict: {reason}"),
                    });
                }
            }
            changed = true;
        }
        if changed {
            let revision = locked.save(&mut state)?;
            self.revision.send_replace(revision);
        }
        Ok(())
    }
}
