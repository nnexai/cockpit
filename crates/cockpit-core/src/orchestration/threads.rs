use super::*;
use super::mutate::resume;
use super::store::{DurableOperationKind, ThreadLinkRecord, ThreadReadRecord};

pub(super) fn validate_thread_id(thread_id: &str) -> Result<(), InspectionError> {
    if thread_id.is_empty() || thread_id.len() > 128
        || thread_id.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err(error("invalid_thread_id", "Thread id must be nonempty, at most 128 bytes, and contain no whitespace or control characters"));
    }
    Ok(())
}

fn owner<'a>(state: &'a OrchestrationState, session: &str, root_id: &str) -> Result<&'a Run, InspectionError> {
    state.runs.iter().find(|run| run.run_id == root_id && run.session_id == session
        && run.root_id == run.run_id && run.parent_run_id.is_none()
        && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
        && run.stage != RunStage::Closed)
        .ok_or_else(|| error("thread_owner_invalid", "Thread owner must be an open, same-session top-level root"))
}

/// Only a settled attachment's exact, locally proven process incarnation can
/// establish absence. A launch or resume may replace the saved identity.
pub(super) fn owner_proven_dead(run: &Run) -> bool {
    if run.stage != RunStage::Active
        || run.resume.as_ref().is_some_and(|resume|
            matches!(resume.phase, ResumePhase::Reconciling | ResumePhase::Launching))
        || run.dispatch.as_ref().is_some_and(|dispatch| dispatch.step != DispatchStep::Launched)
    {
        return false;
    }
    run.attachment.as_ref().is_some_and(|attachment| {
        matches!(attachment.state, AttachmentState::Connected | AttachmentState::Disconnected)
            && attachment.process.as_ref().is_some_and(|process| matches!(
            crate::process_identity::same_machine_process_running(process, attachment.machine_id.as_deref()),
            Ok(false)))
    })
}

fn message_in_thread(message: &Message, thread_id: &str) -> bool {
    message.thread_id.as_deref() == Some(thread_id)
        || (message.thread_id.is_none() && message.message_id == thread_id)
}
/// A worker report is projected only through a durable link to that worker's
/// own task and root. The report itself normally has no thread id.
pub(super) fn linked_worker_interim(state: &OrchestrationState, session: &str, thread_id: &str, message: &Message) -> bool {
    if message.kind != MessageKind::Report
        || !message.report.as_ref().is_some_and(|report| report.kind == ReportKind::Progress
            && WorkerCheckpoint::decode(report.plan.as_deref()).is_some_and(|checkpoint| checkpoint.kind == WorkerCheckpointKind::Interim))
        || message.thread_id.as_deref().is_some_and(|id| id != thread_id) {
        return false;
    }
    let ActorRef::Run { run_id } = &message.from else { return false };
    let Some(run) = state.runs.iter().find(|run| run.run_id == *run_id && run.session_id == session
        && run.kind == RunKind::Worker && run.root_id != run.run_id) else { return false };
    let Some(task_id) = run.task_id.as_deref() else { return false };
    if !state.runs.iter().any(|root| root.run_id == run.root_id && root.root_id == root.run_id
        && root.parent_run_id.is_none() && root.session_id == session)
        || !state.thread_links.iter().any(|link| link.thread_id == thread_id
            && link.root_id == run.root_id && link.task_id == task_id) { return false; }
    let mut parent = run.parent_run_id.as_deref();
    for _ in 0..state.runs.len() {
        let Some(id) = parent else { return false };
        let Some(ancestor) = state.runs.iter().find(|ancestor| ancestor.run_id == id
            && ancestor.root_id == run.root_id && ancestor.session_id == session) else { return false };
        if id == message.to_run_id { return true; }
        parent = ancestor.parent_run_id.as_deref();
    }
    false
}

pub(super) fn current_worker_interim(state: &OrchestrationState, session: &str, thread_id: &str, message: &Message) -> bool {
    if message.stale || !linked_worker_interim(state, session, thread_id, message) { return false; }
    let ActorRef::Run { run_id } = &message.from else { return false };
    let Some(run) = state.runs.iter().find(|run| run.run_id == *run_id && run.session_id == session) else { return false };
    run.stage != RunStage::Closed && run.task_id.as_deref().and_then(|task_id|
        projection::current_task_run(state, &run.root_id, task_id)).is_some_and(|current| current.run_id == *run_id)
}

fn attention_candidate(state: &OrchestrationState, session: &str, thread_id: &str, message: &Message) -> bool {
    if message.kind != MessageKind::Report { return false; }
    if linked_worker_interim(state, session, thread_id, message) { return true; }
    message_in_thread(message, thread_id)
        && message.report.as_ref().is_some_and(|report| report.kind == ReportKind::Result)
        && matches!(&message.from, ActorRef::Run { run_id } if state.runs.iter().any(|run|
            run.run_id == *run_id && run.session_id == session && run.root_id == run.run_id
            && run.parent_run_id.is_none() && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)))
}

/// Resolve the ID in this thread, never selecting an arbitrary sender when
/// two reports reuse an ID.
pub(super) fn read_target<'a>(state: &'a OrchestrationState, session: &str, thread_id: &str, id: &str, sender: Option<&str>) -> Option<&'a Message> {
    let mut matches = state.messages.iter().filter(|message| message.message_id == id
        && attention_candidate(state, session, thread_id, message)
        && match (&message.from, sender) {
            (ActorRef::Run { run_id }, Some(sender)) => run_id == sender && linked_worker_interim(state, session, thread_id, message),
            (_, None) => !linked_worker_interim(state, session, thread_id, message),
            _ => false,
        });
    let target = matches.next()?;
    matches.next().is_none().then_some(target)
}

fn root_result_id_exists(state: &OrchestrationState, session: &str, thread_id: &str, id: &str) -> bool {
    state.messages.iter().any(|message| message.message_id == id
        && attention_candidate(state, session, thread_id, message)
        && !linked_worker_interim(state, session, thread_id, message))
}

/// The marker is opaque to old readers and encodes the complete worker event
/// identity without changing the persisted thread-read schema.
pub(super) fn interim_read_key(state: &OrchestrationState, message: &Message) -> Option<String> {
    let ActorRef::Run { run_id } = &message.from else { return None };
    let run = state.runs.iter().find(|run| run.run_id == *run_id)?;
    let task_id = run.task_id.as_deref()?;
    let identity = serde_json::to_string(&(run_id, &message.to_run_id, &message.message_id, task_id, run.attempt)).ok()?;
    Some(format!("cockpit-interim-read-v1:{identity}"))
}

// Empty thread IDs cannot be produced by mark_read, or by a legacy
// question (whose message ID is nonempty). Keeping the marker outside every
// real thread also prevents old readers interpreting it as a read receipt,
// even if a future message happens to reuse the encoded marker as its ID.
const CLOSE_PREFIX: &str = "cockpit-thread-close-v1:";

pub(super) fn close_marker(session: &str, thread_id: &str, messages: usize, widgets: usize) -> String {
    format!("{CLOSE_PREFIX}{}", serde_json::to_string(&(session, thread_id, messages, widgets)).expect("close marker serializes"))
}

pub(super) fn parse_close_marker(read: &ThreadReadRecord) -> Option<(String, String, usize, usize)> {
    if !read.thread_id.is_empty() || !matches!(read.origin, ActorRef::Operator) { return None; }
    serde_json::from_str(read.through_message_id.strip_prefix(CLOSE_PREFIX)?).ok()
}

pub(super) fn is_read(state: &OrchestrationState, session: &str, thread_id: &str, message: &Message) -> bool {
    if linked_worker_interim(state, session, thread_id, message) {
        let Some(key) = interim_read_key(state, message) else { return false };
        return !root_result_id_exists(state, session, thread_id, &key)
            && state.thread_reads.iter().any(|read| read.thread_id == thread_id && read.through_message_id == key);
    }
    state.thread_reads.iter().filter(|read| read.thread_id == thread_id).any(|read| {
        !state.messages.iter().any(|candidate| linked_worker_interim(state, session, thread_id, candidate)
            && interim_read_key(state, candidate).as_deref() == Some(read.through_message_id.as_str()))
            && read_target(state, session, thread_id, &read.through_message_id, None)
                .is_some_and(|mark| matches!((&mark.from, &message.from),
                    (ActorRef::Run { run_id: left }, ActorRef::Run { run_id: right }) if left == right)
                    && mark.to_run_id == message.to_run_id && mark.seq >= message.seq)
    })
}
/// A report may start a thread, but cannot turn correlation with another
/// owner's thread into authority. Explicit handoffs may name multiple owners.
pub(super) fn check_report_authority(
    state: &OrchestrationState,
    thread_id: &str,
    root_id: &str,
) -> Result<(), InspectionError> {
    let session = state.runs.iter().find(|run| run.run_id == root_id)
        .map(|run| run.session_id.as_str())
        .ok_or_else(|| error("thread_report_forbidden", "Thread owner is unavailable"))?;
    let mut owned = false;
    let mut authorized = false;
    let mut check_owner = |owner_id: &str| -> Result<(), InspectionError> {
        let same_session = state.runs.iter().any(|run| run.run_id == owner_id
            && run.session_id == session && run.root_id == run.run_id && run.parent_run_id.is_none());
        if !same_session {
            return Err(error("thread_report_forbidden", "Thread ownership cannot cross sessions"));
        }
        owned = true;
        authorized |= owner_id == root_id;
        Ok(())
    };
    for link in state.thread_links.iter().filter(|link| link.thread_id == thread_id) {
        check_owner(&link.root_id)?;
    }
    for message in state.messages.iter().filter(|message| message_in_thread(message, thread_id)) {
        if matches!(message.from, ActorRef::Operator) && message.handoff.is_some() {
            let owner_id = state.runs.iter().find(|run| run.run_id == message.to_run_id)
                .map(|run| run.root_id.as_str()).unwrap_or(&message.to_run_id);
            check_owner(owner_id)?;
        } else if message.kind == MessageKind::Report {
            if let ActorRef::Run { run_id } = &message.from {
                if message.to_run_id == *run_id && state.runs.iter().any(|run|
                    run.run_id == *run_id && run.root_id == run.run_id && run.parent_run_id.is_none())
                {
                    check_owner(run_id)?;
                }
            }
        }
    }
    if owned && !authorized {
        return Err(error("thread_report_forbidden", "The thread belongs to another root"));
    }
    Ok(())
}


fn check_link_root(state: &OrchestrationState, thread_id: &str, root_id: &str) -> Result<(), InspectionError> {
    if state.thread_links.iter().any(|link| link.thread_id == thread_id && link.root_id != root_id) {
        return Err(error("thread_root_conflict", "A thread's task links cannot cross roots"));
    }
    Ok(())
}

fn check_link_authority(ctx: &MutationCtx<'_>, root_id: &str, thread_id: &str) -> Result<(), InspectionError> {
    validate_thread_id(thread_id)?;
    mutate::tasks::task_root_authority(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id)?;
    check_link_root(ctx.state, thread_id, root_id)?;
    let rooted = ctx.state.messages.iter().any(|message| {
        message_in_thread(message, thread_id) && (
            (message.to_run_id == root_id && matches!(message.from, ActorRef::Operator)
                && message.handoff.is_some())
            || (matches!(ctx.actor, Actor::Agent(_)) && message.kind == MessageKind::Report
                && matches!(&message.from, ActorRef::Run { run_id } if run_id == root_id))
        )
    });
    if !rooted {
        return Err(error("thread_link_forbidden", "The thread is not rooted at this owner for this actor"));
    }
    Ok(())
}

fn insert_link(state: &mut OrchestrationState, thread_id: &str, root_id: &str, task_id: &str, linked_by: ActorRef) -> bool {
    if state.thread_links.iter().any(|link| link.thread_id == thread_id && link.root_id == root_id && link.task_id == task_id) {
        return false;
    }
    state.thread_links.push(ThreadLinkRecord {
        thread_id: thread_id.to_owned(), root_id: root_id.to_owned(), task_id: task_id.to_owned(),
        linked_by, at: now(),
    });
    true
}

/// Publish a task-only thread for an authorized worker's first interim when
/// no existing thread links its task. Never attach it to another conversation.
pub(super) fn link_interim_task(state: &mut OrchestrationState, run_index: usize) {
    let run = &state.runs[run_index];
    let Some(task_id) = run.task_id.clone() else { return };
    let root_id = run.root_id.clone();
    let run_id = run.run_id.clone();
    if state.thread_links.iter().any(|link| link.root_id == root_id && link.task_id == task_id) {
        return;
    }
    let thread_id = loop {
        let candidate = id();
        if !state.thread_links.iter().any(|link| link.thread_id == candidate)
            && !state.thread_reads.iter().any(|read| read.thread_id == candidate)
            && !state.widget_publications.iter().any(|publication| publication.thread_id == candidate)
            && !state.messages.iter().any(|message| message.thread_id.as_deref() == Some(candidate.as_str())
                || (message.thread_id.is_none() && message.message_id == candidate))
        {
            break candidate;
        }
    };
    insert_link(state, &thread_id, &root_id, &task_id, ActorRef::Run { run_id });
}

/// Journal the link before the caller publishes the task document.
pub(super) fn journal_task_link(ctx: &mut MutationCtx<'_>, root_id: &str, task_id: &str, thread_id: &str) -> Result<(), InspectionError> {
    check_link_authority(ctx, root_id, thread_id)?;
    let by = actor_ref(ctx.actor, ctx.caller, ctx.state);
    if insert_link(ctx.state, thread_id, root_id, task_id, by) {
        ctx.locked.save(ctx.state)?;
        ctx.service.revision.send_replace(ctx.state.revision);
    }
    Ok(())
}

pub(super) fn link(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::ThreadLink { root_id, task_id, thread_id } = action else { unreachable!() };
    check_link_authority(ctx, &root_id, &thread_id)?;
    let document = ctx.locked.tasks(&root_id)?;
    document.task(&task_id)?;
    let by = actor_ref(ctx.actor, ctx.caller, ctx.state);
    if insert_link(ctx.state, &thread_id, &root_id, &task_id, by) {
        Ok(Applied::Machine(OrchestrationActionResult::Done))
    } else {
        Ok(Applied::Document(OrchestrationActionResult::Done))
    }
}

pub(super) fn mark_read(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    operator(ctx.actor)?;
    let OrchestrationAction::ThreadMarkRead { thread_id, through_message_id, run_id: sender } = action else { unreachable!() };
    validate_thread_id(&thread_id)?;
    let Some(message) = read_target(ctx.state, ctx.session_id, &thread_id, &through_message_id, sender.as_deref()) else {
        return Err(error("thread_result_invalid", "Read mark must identify an unambiguous attention report in this thread"));
    };
    let ActorRef::Run { run_id } = &message.from else { unreachable!() };
    let is_interim = sender.is_some();
    let valid = if is_interim {
        current_worker_interim(ctx.state, ctx.session_id, &thread_id, message)
    } else {
        !message.stale && !ctx.state.messages.iter().skip_while(|candidate| !std::ptr::eq(*candidate, message)).skip(1).any(|later|
            !later.stale && message_in_thread(later, &thread_id)
                && matches!(&later.from, ActorRef::Run { run_id: sender } if sender == run_id)
                && later.report.as_ref().is_some_and(|report| matches!(report.kind, ReportKind::NeedsInput | ReportKind::Result)))
    };
    if !valid {
        return Err(error("thread_result_invalid", "Read mark must identify a current attention report in this thread"));
    }
    if is_read(ctx.state, ctx.session_id, &thread_id, message) {
        return Ok(Applied::Document(OrchestrationActionResult::Done));
    }
    let read_id = if is_interim {
        let key = interim_read_key(ctx.state, message).ok_or_else(|| error("thread_result_invalid", "Interim identity is unavailable"))?;
        if root_result_id_exists(ctx.state, ctx.session_id, &thread_id, &key) {
            return Err(error("thread_result_invalid", "Interim read identity is ambiguous"));
        }
        key
    } else { through_message_id };
    ctx.state.thread_reads.push(ThreadReadRecord { thread_id, through_message_id: read_id, origin: ActorRef::Operator, at: now() });
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

pub(super) fn close(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    if operator(ctx.actor)? != OperatorOrigin::Native {
        return Err(error("actor_forbidden", "Thread close requires a local native operator"));
    }
    let OrchestrationAction::ThreadClose { thread_id } = action else { unreachable!() };
    validate_thread_id(&thread_id)?;
    let view = inbox_view::project_uncapped(
        &InboxRequest { session_id: ctx.session_id.into() }, ctx.state, vec![],
        Err(error("offline", "Runtime observation unavailable")), &now(), "")?;
    let thread = view.threads.iter().find(|thread| thread.thread_id == thread_id)
        .ok_or_else(|| error("thread_close_invalid", "Thread is not in this session's Inbox"))?;
    // Projection displays only the latest owner report, but a later Progress
    // does not supersede a Result for mark_read. Do not close over it.
    let unread_result = ctx.state.messages.iter().enumerate().any(|(index, message)| {
        if message.stale || !attention_candidate(ctx.state, ctx.session_id, &thread_id, message)
            || linked_worker_interim(ctx.state, ctx.session_id, &thread_id, message)
            || !message.report.as_ref().is_some_and(|report| report.kind == ReportKind::Result)
            || is_read(ctx.state, ctx.session_id, &thread_id, message) { return false; }
        let ActorRef::Run { run_id } = &message.from else { return false };
        !ctx.state.messages[index + 1..].iter().any(|later|
            !later.stale && message_in_thread(later, &thread_id)
                && matches!(&later.from, ActorRef::Run { run_id: sender } if sender == run_id)
                && later.report.as_ref().is_some_and(|report| matches!(report.kind, ReportKind::NeedsInput | ReportKind::Result)))
    });
    if unread_result {
        return Err(error("thread_close_invalid", "Answer or acknowledge first"));
    }
    match thread.state {
        InboxState::NeedsYou => return Err(error("thread_close_invalid", "Answer or acknowledge first")),
        InboxState::Handled => return Ok(Applied::Document(OrchestrationActionResult::Done)),
        InboxState::Underway => {}
    }
    let marker = close_marker(ctx.session_id, &thread_id, ctx.state.messages.len(), ctx.state.widget_publications.len());
    ctx.state.thread_reads.retain(|read| !parse_close_marker(read).is_some_and(|(session, id, _, _)|
        session == ctx.session_id && id == thread_id));
    ctx.state.thread_reads.push(ThreadReadRecord {
        thread_id: String::new(), through_message_id: marker, origin: ActorRef::Operator, at: now(),
    });
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

pub(super) fn handoff(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let origin = operator(ctx.actor)?;
    let OrchestrationAction::ThreadHandoff { message_id, thread_id, to_run_id, text, task_id, expected_task_revision, binding, images, recipient } = &action else { unreachable!() };
    if let Some(result) = resume::replay(ctx, message_id, DurableOperationKind::ThreadHandoff, &action)? {
        return Ok(Applied::Document(result));
    }
    validate_thread_id(thread_id)?;
    let target = owner(ctx.state, ctx.session_id, to_run_id)?;
    let existing_thread = ctx.state.messages.iter().any(|message| message_in_thread(message, thread_id));
    if !existing_thread && owner_proven_dead(target) {
        return Err(error("thread_owner_invalid", "A new thread cannot be handed off to a proven-dead owner"));
    }
    let images = messages::check_images(ctx.state, ctx.actor, None, images, ctx.resolved)?;
    if text.trim().is_empty() && images.is_empty() { return Err(error("invalid_message", "Handoff requires text or an image")); }
    bounded(text, 16 * 1024)?;
    let resolved_binding = if let Some(requested) = binding {
        if requested.repository_id.is_none() && requested.workspace_id.is_none()
            && requested.branch.is_none() && requested.base_ref.is_none() {
            return Err(error("invalid_binding", "Handoff binding must contain an explicit field"));
        }
        let resolved = ctx.resolved.and_then(|input| input.handoff_binding.as_ref())
            .filter(|record| &record.requested == requested)
            .filter(|record| requested.repository_id.is_none() || record.repository_common_dir.is_some())
            .filter(|record| requested.workspace_id.is_none() || record.workspace_checkout.is_some())
            .ok_or_else(|| error("handoff_binding_unresolved", "The explicit handoff binding was not resolved by the host"))?;
        Some(resolved.clone())
    } else { None };
    if !existing_thread && !Uuid::parse_str(thread_id).is_ok_and(|id| id.get_version_num() == 4) {
        return Err(error("invalid_thread_id", "A new operator thread id must be a UUIDv4"));
    }
    let (revision, attempt) = if let Some(task_id) = task_id {
        check_link_root(ctx.state, thread_id, to_run_id)?;
        let document = ctx.locked.tasks(to_run_id)?;
        let task = document.task(task_id)?;
        if expected_task_revision.as_deref() != Some(task.task_revision.as_str()) {
            return Err(error("task_revision_conflict", "Task changed or its revision was not supplied"));
        }
        (Some(task.task_revision.clone()), projection::current_task_run(ctx.state, to_run_id, task_id).map(|run| run.run_id.clone()))
    } else {
        if expected_task_revision.is_some() { return Err(error("task_revision_conflict", "Task revision requires a task")); }
        (None, None)
    };
    let worker = if *recipient == HandoffRecipient::TaskWorker {
        let task_id = task_id.as_ref().ok_or_else(|| error("task_worker_unavailable", "A task is required for a worker handoff"))?;
        let run = projection::current_task_run(ctx.state, to_run_id, task_id)
            .filter(|run| run.kind == RunKind::Worker && run.stage != RunStage::Closed)
            .ok_or_else(|| error("task_worker_unavailable", "The task has no current open worker"))?;
        Some(run.run_id.clone())
    } else { None };
    let directive_id = if worker.is_some() {
        Some(directives::record_task_directive(ctx.state, to_run_id, task_id.as_deref().expect("worker task"),
            text, thread_id, message_id)?)
    } else { None };
    let destination = worker.as_deref().unwrap_or(to_run_id);
    let result = messages::append(ctx.state, messages::AppendMessage {
        from: ActorRef::Operator, to_run_id: destination, message_id,
        kind: if worker.is_some() { MessageKind::UserDirective } else { MessageKind::Instruction }, text,
        in_reply_to: None, report: None, thread_id: Some(thread_id.clone()),
        handoff: Some(HandoffRecord { task_id: task_id.clone(), task_revision: revision,
            attempt_run_id: attempt, binding: resolved_binding, origin }),
        images,
        stale: false, escalated_from: None,
    })?;
    if let Some(directive_id) = directive_id {
        let copy_id = format!("directive-copy:{directive_id}");
        messages::append(ctx.state, messages::AppendMessage {
            from: ActorRef::Dispatcher, to_run_id, message_id: &copy_id, kind: MessageKind::Observation, text,
            in_reply_to: None, report: None, thread_id: Some(thread_id.clone()), handoff: None,
            images: Vec::new(), stale: false, escalated_from: None,
        })?;
    }
    if let Some(task_id) = task_id {
        insert_link(ctx.state, thread_id, to_run_id, task_id, ActorRef::Operator);
    }
    resume::record(ctx, message_id.clone(), DurableOperationKind::ThreadHandoff, &action,
        to_run_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}
