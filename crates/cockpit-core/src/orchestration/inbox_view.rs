use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cockpit_protocol::orchestration::*;
use super::{cleanup, herdr::RuntimeView, projection, store::OrchestrationState};
use crate::InspectionError;

fn location(observation: Option<&RunObservation>) -> Option<InboxLocation> {
    let o = observation.filter(|o| o.presence == Presence::Present)?;
    Some(InboxLocation { workspace_id: o.workspace_id.clone()?, workspace_label: o.workspace_label.clone()?,
        tab_id: o.tab_id.clone()?, tab_label: o.tab_label.clone()?, pane_id: o.pane_id.clone()? })
}

fn owner_source(run: &Run, card: bool) -> InboxSource {
    if card { InboxSource::OwnerCard { run_id: run.run_id.clone(), harness: run.launch_config.harness_kind.clone() } }
    else { InboxSource::Owner { run_id: run.run_id.clone(), harness: run.launch_config.harness_kind.clone() } }
}
fn text(value: String, source: InboxSource, message: Option<&Message>) -> InboxText {
    InboxText { text: value, source, message_id: message.map(|m| m.message_id.clone()), at: message.map(|m| m.created_at.clone()), images: Vec::new() }
}

fn task_lane_label(lane: TaskLane) -> &'static str {
    match lane {
        TaskLane::Blocked => "Blocked",
        TaskLane::Ready => "Ready",
        TaskLane::Working => "Assigned",
        TaskLane::Review => "Review",
        TaskLane::Accepted => "Accepted",
    }
}
fn inbox_task_label(task: &InboxTaskLink) -> &'static str {
    match task.disposition.as_ref().map(|disposition| disposition.kind) {
        Some(TaskDispositionKind::CompletedElsewhere) => "Set aside · completed elsewhere",
        Some(TaskDispositionKind::Abandoned) => "Set aside · abandoned",
        None => task_lane_label(task.lane),
    }
}
fn integration_fact(run: &Run) -> Option<InboxText> {
    let (value, source) = match (&run.integration, &run.source_output) {
        (Integration::Integrated { .. }, _) => ("Integration: Integrated", InboxSource::Recorded),
        (Integration::NotApplicable { .. }, _) => ("Integration: Not applicable", InboxSource::Recorded),
        (Integration::Pending, Some(source @ SourceOutput::Code { result_message_id, .. }))
            if run.stage == RunStage::Closed && run.close_reason == Some(CloseReason::Accepted)
                && run.result.as_ref().is_some_and(|result| result.outcome == Some(ReportOutcome::Succeeded)
                    && &result.message_id == result_message_id && result.source_output.as_ref() == Some(source)) =>
            ("Integration: Pending", InboxSource::Recorded),
        (Integration::Pending, Some(SourceOutput::NonCode { .. })) =>
            ("Git integration not needed for non-code output", InboxSource::Worker { run_id: run.run_id.clone() }),
        (Integration::Pending, _) => return None,
    };
    Some(text(value.into(), source, None))
}
fn message_text(value: String, source: InboxSource, message: &Message) -> InboxText {
    let mut result = text(value, source, Some(message));
    result.images = message.images.clone();
    result
}
fn build_request_echo(message: &Message) -> InboxRequestEcho {
    InboxRequestEcho { message_id: message.message_id.clone(), text: message.text.clone(), created_at: message.created_at.clone(), to_run_id: message.to_run_id.clone(), handoff: message.handoff.clone().unwrap(), images: message.images.clone() }
}
fn first_line(value: &str) -> String {
    let line = value.lines().next().unwrap_or("");
    let mut end = line.len().min(80);
    while !line.is_char_boundary(end) { end -= 1; }
    line[..end].to_owned()
}
fn message_order<'a>(m: &'a Message) -> (&'a str, &'a str, u64, &'a str) {
    (&m.created_at, &m.to_run_id, m.seq, &m.message_id)
}
fn report_owner<'a>(m: &Message, roots: &HashMap<&str, &'a Run>) -> Option<&'a Run> {
    if m.stale || m.kind != MessageKind::Report || m.report.is_none() { return None; }
    let ActorRef::Run { run_id } = &m.from else { return None };
    roots.get(run_id.as_str()).copied()
}
fn interim_ref(state: &OrchestrationState, message: &Message) -> Option<InboxInterimRef> {
    let ActorRef::Run { run_id } = &message.from else { return None };
    let run = state.runs.iter().find(|run| run.run_id == *run_id)?;
    Some(InboxInterimRef { run_id: run_id.clone(), message_id: message.message_id.clone(),
        task_id: run.task_id.clone()?, attempt: run.attempt })
}


fn timeline(messages: &[&Message], state: &OrchestrationState, thread_id: &str, roots: &HashMap<&str, &Run>, boards: &[TaskBoard]) -> (Vec<InboxEntry>, u32, Vec<InboxEntry>, Option<u32>) {
    let all = &state.messages;
    let positions: HashMap<*const Message, usize> = all.iter().enumerate()
        .map(|(index, message)| (message as *const Message, index)).collect();
    let mut entries = messages.to_vec();
    // Answers can predate thread attribution. Match both recipient and question ID:
    // message IDs are not globally unique across owner inboxes.
    for answer in all.iter().filter(|m| !m.stale && m.kind == MessageKind::Answer
        && (matches!(m.from, ActorRef::Operator) || matches!(&m.from, ActorRef::Run { run_id }
            if run_id == &m.to_run_id && roots.contains_key(run_id.as_str())))) {
        if messages.iter().any(|question| report_owner(question, roots).is_some_and(|root|
            root.run_id == answer.to_run_id && question.report.as_ref().is_some_and(|r|
                r.kind == ReportKind::NeedsInput && answer.in_reply_to.as_deref() == Some(question.message_id.as_str()))))
            && !entries.iter().any(|m| std::ptr::eq(*m, answer)) {
            entries.push(answer);
        }
    }
    entries.sort_by_key(|m| positions[&(*m as *const Message)]);
    let mut had_request = false;
    let mut result = Vec::new();
    for m in entries.into_iter().filter(|m| !m.stale || super::threads::linked_worker_interim(state,
        roots.values().next().map_or("", |root| root.session_id.as_str()), thread_id, m)) {
        let (kind, value, source, outcome) = if matches!(m.from, ActorRef::Operator) {
            let kind = if m.kind == MessageKind::Instruction && m.handoff.is_some() {
                let kind = if had_request { InboxEntryKind::FollowUp } else { InboxEntryKind::Request };
                had_request = true;
                kind
            } else if m.kind == MessageKind::Answer { InboxEntryKind::Answer }
            else if m.kind == MessageKind::UserDirective { InboxEntryKind::Directive }
            else { continue };
            (kind, m.text.clone(), InboxSource::User, None)
        } else if m.kind == MessageKind::Answer && matches!(&m.from, ActorRef::Run { run_id }
            if run_id == &m.to_run_id && roots.contains_key(run_id.as_str())) {
            (InboxEntryKind::Answer, m.text.clone(), owner_source(roots[m.to_run_id.as_str()], false), None)
        } else if let Some(root) = report_owner(m, roots) {
            let report = m.report.as_ref().unwrap();
            let kind = match report.kind {
                ReportKind::Progress => InboxEntryKind::Update,
                ReportKind::Result => InboxEntryKind::Result,
                ReportKind::NeedsInput => InboxEntryKind::Question,
                ReportKind::Initialized => continue,
            };
            (kind, report.summary.clone(), owner_source(root, false), if report.kind == ReportKind::Result { report.outcome } else { None })
        } else if super::threads::linked_worker_interim(state,
            roots.values().next().map_or("", |root| root.session_id.as_str()), thread_id, m) {
            let ActorRef::Run { run_id } = &m.from else { unreachable!() };
            (InboxEntryKind::Interim, m.report.as_ref().unwrap().summary.clone(),
                InboxSource::Worker { run_id: run_id.clone() }, None)
        } else { continue };
        let position = positions[&(m as *const Message)];
        let task_title = if m.kind == MessageKind::UserDirective {
            state.runs.iter().find(|run| run.run_id == m.to_run_id)
                .and_then(|run| boards.iter().find(|board| board.root_id == run.root_id))
                .and_then(|board| m.handoff.as_ref().and_then(|handoff| handoff.task_id.as_ref())
                    .and_then(|id| board.tasks.iter().find(|task| &task.task.task_id == id)))
                .map(|task| task.task.title.clone())
        } else { None };
        let interim = (kind == InboxEntryKind::Interim).then(|| interim_ref(state, m)).flatten();
        let artifacts = interim.as_ref().and_then(|identity| {
            let run = state.runs.iter().find(|run| run.run_id == identity.run_id)?;
            let checkpoint = WorkerCheckpoint::decode(m.report.as_ref()?.plan.as_deref())?;
            Some(super::artifacts::artifacts_for_run(run, &state.messages).into_iter()
                .filter(|artifact| checkpoint.artifacts.contains(&artifact.path)).collect())
        }).unwrap_or_default();
        result.push(((position, 1, 0), InboxEntry { message_id: m.message_id.clone(), at: m.created_at.clone(), kind,
            text: value, source, outcome, images: m.images.clone(), in_reply_to: m.in_reply_to.clone(), interim, artifacts, widget: None, task_title }));
    }
    for (index, publication) in state.widget_publications.iter().enumerate().filter(|(_, p)|
        p.thread_id == thread_id && roots.contains_key(p.root_id.as_str())) {
        let root = roots[publication.root_id.as_str()];
        let event = &publication.event;
        result.push(((publication.message_position, 0, index), InboxEntry {
            message_id: event.publication_id.clone(), at: publication.at.clone(), kind: InboxEntryKind::Widget,
            text: event.title.clone(), source: if event.binding.run_id == root.run_id { owner_source(root, false) }
                else { InboxSource::Worker { run_id: event.binding.run_id.clone() } }, outcome: None,
            images: Vec::new(), in_reply_to: None, interim: None, artifacts: Vec::new(), widget: Some(event.clone()), task_title: None,
        }));
    }
    result.sort_by_key(|(order, _)| *order);
    let omitted = result.len().saturating_sub(120);
    let mut earlier_widgets: Vec<_> = result.drain(..omitted).filter_map(|(_, entry)|
        (entry.kind == InboxEntryKind::Widget).then_some(entry)).collect();
    let earlier_omitted = earlier_widgets.len().saturating_sub(120);
    earlier_widgets.drain(..earlier_omitted);
    (result.into_iter().map(|(_, entry)| entry).collect(), omitted as u32, earlier_widgets,
        (earlier_omitted > 0).then_some(earlier_omitted as u32))
}

fn task_evidence(state: &OrchestrationState, root_id: &str, task_id: &str, current: Option<&Run>) -> Option<InboxTaskEvidence> {
    let session = state.runs.iter().find(|r| r.run_id == root_id && r.run_id == r.root_id)?.session_id.as_str();
    let runs: Vec<&Run> = state.runs.iter().filter(|r| r.session_id == session && r.root_id == root_id
        && r.task_id.as_deref() == Some(task_id) && r.kind != RunKind::Supervisor).collect();
    let run = runs.iter().copied().filter(|r| r.result.is_some())
        .max_by(|a, b| (a.attempt, &a.created_at, &a.run_id).cmp(&(b.attempt, &b.created_at, &b.run_id))).or(current)?;
    let (checks, commits, head_commit) = match &run.source_output {
        Some(SourceOutput::Code { focused_checks, commits, head_commit, .. }) =>
            (focused_checks.clone(), commits.clone(), Some(head_commit.clone())),
        Some(SourceOutput::NonCode { focused_checks, .. }) => (focused_checks.clone(), vec![], None),
        None => (vec![], vec![], None),
    };
    let integration = match &run.integration {
        Integration::Integrated { target_ref, target_commit, .. } => Some(InboxTaskIntegration {
            target: target_ref.strip_prefix("refs/heads/").unwrap_or(target_ref).to_owned(), commit: target_commit.clone() }),
        _ => None,
    };
    let run_ids: HashSet<&str> = runs.iter().map(|r| r.run_id.as_str()).collect();
    let mut reports: Vec<&Message> = state.messages.iter().filter(|m| !m.stale && m.kind == MessageKind::Report
        && matches!(&m.from, ActorRef::Run { run_id } if run_ids.contains(run_id.as_str()))).collect();
    reports.sort_by_key(|m| message_order(m));
    let mut seen = HashSet::new();
    let images = reports.iter().rev().flat_map(|m| m.images.iter())
        .filter(|image| seen.insert(image.image_id.as_str())).take(12).cloned().collect();
    let mut decisions: Vec<_> = state.messages.iter().rev().filter(|m| !m.stale && m.kind == MessageKind::Report
        && matches!(&m.from, ActorRef::Run { run_id } if run_ids.contains(run_id.as_str()))).filter_map(|question| {
        let report = question.report.as_ref().filter(|r| r.kind == ReportKind::NeedsInput)?;
        let ActorRef::Run { run_id } = &question.from else { return None };
        let answer = state.messages.iter().filter(|m| !m.stale && m.kind == MessageKind::Answer
            && m.to_run_id == *run_id && m.in_reply_to.as_deref() == Some(report.message_id.as_str()))
            .max_by_key(|m| m.seq);
        let by = answer.map(|m| match &m.from {
            ActorRef::Operator => InboxDecisionBy::You,
            ActorRef::Dispatcher => InboxDecisionBy::Cockpit,
            ActorRef::Run { run_id } if run_id == root_id => InboxDecisionBy::Owner,
            ActorRef::Run { .. } => InboxDecisionBy::Agent,
        });
        Some(InboxDecision { question: report.summary.clone(), answer: answer.map(|m| m.text.clone()),
            asked_at: question.created_at.clone(), answered_at: answer.map(|m| m.created_at.clone()), by })
    }).take(20).collect();
    decisions.reverse();
    Some(InboxTaskEvidence { run_id: run.run_id.clone(), attempt: run.attempt, attempts: runs.len() as u32,
        result: run.result.as_ref().map(|r| InboxTaskResult { summary: r.summary.clone(), outcome: r.outcome, at: r.at.clone() }),
        checks, commits, head_commit, integration, artifacts: super::artifacts::artifacts_for_run(run, &state.messages), images, decisions })
}

fn request_title(value: &str) -> String {
    let line = value.lines().next().unwrap_or("");
    if line.chars().count() <= 80 { return line.to_owned(); }
    let prefix: String = line.chars().take(79).collect();
    let boundary = prefix.rfind(char::is_whitespace).unwrap_or(prefix.len());
    format!("{}…", prefix[..boundary].trim_end())
}

fn question_message<'a>(messages: &'a [Message], q: &QuestionStatus) -> Option<&'a Message> {
    messages.iter().find(|m| m.message_id == q.question_message_id
        && matches!(&m.from, ActorRef::Run { run_id } if run_id == &q.run_id)
        && m.thread_id == q.thread_id)
}

pub(crate) fn project(request: &InboxRequest, state: &OrchestrationState, boards: Vec<TaskBoard>,
    runtime: Result<RuntimeView, InspectionError>, now: &str, store_id: &str) -> Result<InboxView, InspectionError> {
    project_inner(request, state, boards, runtime, now, store_id, true)
}

pub(super) fn project_uncapped(request: &InboxRequest, state: &OrchestrationState, boards: Vec<TaskBoard>,
    runtime: Result<RuntimeView, InspectionError>, now: &str, store_id: &str) -> Result<InboxView, InspectionError> {
    project_inner(request, state, boards, runtime, now, store_id, false)
}

fn project_inner(request: &InboxRequest, state: &OrchestrationState, boards: Vec<TaskBoard>,
    runtime: Result<RuntimeView, InspectionError>, now: &str, store_id: &str, cap_handled: bool) -> Result<InboxView, InspectionError> {
    let roots: Vec<&Run> = state.runs.iter().filter(|r| r.session_id == request.session_id
        && r.run_id == r.root_id && r.parent_run_id.is_none()).collect();
    let root_map: HashMap<&str, &Run> = roots.iter().map(|r| (r.run_id.as_str(), *r)).collect();
    let message_positions: HashMap<*const Message, usize> = state.messages.iter().enumerate()
        .map(|(index, message)| (message as *const Message, index)).collect();
    let runs: Vec<&Run> = state.runs.iter().filter(|r| r.session_id == request.session_id && root_map.contains_key(r.root_id.as_str())).collect();
    let intents: HashSet<&str> = state.task_intents.iter().filter(|i| root_map.contains_key(i.root_id.as_str())).map(|i| i.run_id.as_str()).collect();
    let mut questions: BTreeMap<&str, Vec<QuestionStatus>> = BTreeMap::new();
    for root in &roots {
        let inbox: Vec<&Message> = state.messages.iter().filter(|m| m.to_run_id == root.run_id).collect();
        for q in projection::question_statuses(root, &inbox, &state.messages) {
            // Borrow the thread key from the authoritative sender's message.
            if let Some(m) = question_message(&state.messages, &q) {
                let key = m.thread_id.as_deref().unwrap_or(m.message_id.as_str());
                questions.entry(key).or_default().push(q);
            }
        }
    }
    let mut groups: BTreeMap<&str, Vec<&Message>> = BTreeMap::new();
    for m in &state.messages {
        let owner_report = report_owner(m, &root_map).is_some();
        let worker_directive = m.kind == MessageKind::UserDirective && matches!(m.from, ActorRef::Operator)
            && runs.iter().any(|run| run.run_id == m.to_run_id && root_map.contains_key(run.root_id.as_str()));
        if (root_map.contains_key(m.to_run_id.as_str()) && matches!(m.from, ActorRef::Operator)) || owner_report || worker_directive {
            if let Some(id) = m.thread_id.as_deref() { groups.entry(id).or_default().push(m); }
        }
    }
    for link in &state.thread_links {
        if !root_map.contains_key(link.root_id.as_str()) { continue; }
        for message in state.messages.iter().filter(|message|
            super::threads::linked_worker_interim(state, &request.session_id, &link.thread_id, message)) {
            let group = groups.entry(link.thread_id.as_str()).or_default();
            if !group.iter().any(|entry| std::ptr::eq(*entry, message)) { group.push(message); }
        }
    }
    for (&id, statuses) in &questions {
        let group = groups.entry(id).or_default();
        for q in statuses {
            if let Some(m) = question_message(&state.messages, q) {
                if !group.iter().any(|entry| entry.message_id == m.message_id
                    && matches!(&entry.from, ActorRef::Run { run_id } if run_id == &q.run_id)) { group.push(m); }
            }
        }
    }
    let runtime_ref = runtime.as_ref().ok();
    let mut task_projects = HashMap::new();
    for link in &state.thread_links {
        if groups.contains_key(link.thread_id.as_str()) && root_map.contains_key(link.root_id.as_str()) {
            task_projects.entry((link.root_id.as_str(), link.task_id.as_str())).or_insert_with(|| projection::task_project(state, &link.root_id, &link.task_id, runtime_ref));
        }
    }
    for messages in groups.values() { for m in messages {
        if let Some(task) = m.handoff.as_ref().and_then(|h| h.task_id.as_deref()) {
            if let Some(run) = runs.iter().find(|run| run.run_id == m.to_run_id) {
                task_projects.entry((run.root_id.as_str(), task)).or_insert_with(|| projection::task_project(state, &run.root_id, task, runtime_ref));
            }
        }
    }}
    let (observation, unmanaged_agents) = projection::observe(&runs, runtime, now);
    let observations: HashMap<&str, &RunObservation> = match &observation {
        RuntimeObservation::Fresh { runs, .. } => runs.iter().map(|o| (o.run_id.as_str(), o)).collect(),
        RuntimeObservation::Unavailable { .. } => HashMap::new(),
    };
    let owners = roots.iter().map(|r| {
        let o = observations.get(r.run_id.as_str()).copied();
        let availability = if r.stage != RunStage::Active && r.stage != RunStage::Closed { OwnerAvailability::Starting }
            else if super::threads::owner_proven_dead(r) { OwnerAvailability::Absent }
            else { match o.map(|o| o.presence) { Some(Presence::Present) => OwnerAvailability::Available,
                _ => OwnerAvailability::Unknown } };
        InboxOwner { root_id: r.run_id.clone(), label: r.label.clone(), kind: r.kind, stage: r.stage,
            run_id: r.run_id.clone(), attempt: r.attempt, session_id: r.session_id.clone(),
            attachment_id: r.attachment.as_ref().map(|a| a.attachment_id.clone()),
            attachment_epoch: r.attachment.as_ref().map(|a| a.epoch),
            harness: r.launch_config.harness_kind.clone(), availability, location: location(o),
            undelivered: state.messages.iter().filter(|m| m.to_run_id == r.run_id && matches!(m.from, ActorRef::Operator) && m.stage == DeliveryStage::Stored).count() as u32 }
    }).collect();
    let cleanups = cleanup::summaries(state, &request.session_id);
    let mut threads = Vec::new();
    for (id, mut messages) in groups {
        messages.sort_by_key(|m| message_order(m));
        let mut owner_ids = BTreeSet::new();
        for m in &messages {
            if m.handoff.is_some() && root_map.contains_key(m.to_run_id.as_str()) { owner_ids.insert(m.to_run_id.as_str()); }
            if let Some(root) = report_owner(m, &root_map) { owner_ids.insert(root.run_id.as_str()); }
            if super::threads::linked_worker_interim(state, &request.session_id, id, m) {
                if let ActorRef::Run { run_id } = &m.from {
                    if let Some(run) = runs.iter().find(|run| run.run_id == *run_id) { owner_ids.insert(run.root_id.as_str()); }
                }
            }
        }
        let mut handoffs = messages.iter().copied().filter(|m| matches!(m.from, ActorRef::Operator) && m.kind == MessageKind::Instruction && m.handoff.is_some());
        let request_message = handoffs.next();
        let request_echo = request_message.map(build_request_echo);
        let follow_ups = handoffs.map(build_request_echo).collect();
        let reports: Vec<(&Message, &Run)> = messages.iter().filter_map(|m| report_owner(m, &root_map).map(|r| (*m, r))).collect();
        let latest_report = reports.last().copied();
        let latest_card = reports.iter().rev().find(|(m, _)| m.report.as_ref().and_then(|r| r.card.as_ref()).is_some()).copied();
        let mut projected_questions = Vec::new();
        if let Some(statuses) = questions.get(id) { for q in statuses {
            if let (Some(m), Some(root)) = (question_message(&state.messages, q), root_map.get(q.run_id.as_str())) {
                if let Some(report) = &m.report { projected_questions.push((message_positions.get(&(m as *const Message)).copied(), InboxQuestion { run_id: q.run_id.clone(), question_message_id: q.question_message_id.clone(), asked_at: q.asked_at.clone(), text: message_text(report.summary.clone(), owner_source(root, false), m), choices: report.card.as_ref().map(|c| c.choices.clone()).unwrap_or_default(), receipt: q.receipt.clone() })); }
            }
        }}
        // Missing legacy positions sort last, with identity as a deterministic fallback.
        projected_questions.sort_by(|(a_pos, a), (b_pos, b)|
            (a_pos.unwrap_or(usize::MAX), &a.run_id, &a.question_message_id)
                .cmp(&(b_pos.unwrap_or(usize::MAX), &b.run_id, &b.question_message_id)));
        let projected_questions: Vec<_> = projected_questions.into_iter().map(|(_, question)| question).collect();
        let receipts: Vec<InboxReceipt> = messages.iter().filter(|m| matches!(m.from, ActorRef::Operator)).map(|m| InboxReceipt { message_id: m.message_id.clone(), kind: m.kind, to_run_id: m.to_run_id.clone(), attempt_run_id: m.handoff.as_ref().and_then(|h| h.attempt_run_id.clone()), stage: m.stage, created_at: m.created_at.clone(), acked_at: m.acked_at.clone(), stale: m.stale }).collect();
        let mut links = BTreeSet::new();
        for link in &state.thread_links { if link.thread_id == id && root_map.contains_key(link.root_id.as_str()) { links.insert((link.root_id.as_str(), link.task_id.as_str())); } }
        for m in &messages { if let Some(task) = m.handoff.as_ref().and_then(|h| h.task_id.as_deref()) {
            if let Some(run) = runs.iter().find(|run| run.run_id == m.to_run_id) { links.insert((run.root_id.as_str(), task)); }
        } }
        let link_count = links.len();
        let mut tasks = Vec::new();
        for (root_id, task_id) in links {
            let Some(view) = boards.iter().find(|b| b.root_id == root_id).and_then(|b| b.tasks.iter().find(|t| t.task.task_id == task_id)) else { continue };
            let current = projection::current_task_run(state, root_id, task_id);
            let lane = projection::lane(&view.task, &view.dependencies, current, &intents);
            let run = current.map(|r| {
                let o = observations.get(r.run_id.as_str()).copied();
                let report_images = state.messages.iter().filter(|m| !m.stale && m.kind == MessageKind::Report && !m.images.is_empty()
                    && matches!(&m.from, ActorRef::Run { run_id } if run_id == &r.run_id))
                    .max_by(|a, b| message_order(a).cmp(&message_order(b))).map(|m| m.images.clone()).unwrap_or_default();
                InboxRun { run_id: r.run_id.clone(), attempt: r.attempt, stage: r.stage, harness: r.launch_config.harness_kind.clone(), session_id: r.session_id.clone(), attachment_id: r.attachment.as_ref().map(|a| a.attachment_id.clone()), attachment_epoch: r.attachment.as_ref().map(|a| a.epoch), presence: o.map_or(Presence::Unobserved, |o| o.presence), agent_status: o.and_then(|o| o.agent_status.clone()), location: location(o), cleanup: cleanups.iter().find(|c| c.run_id == r.run_id).map(|c| c.items.clone()).unwrap_or_default(), report_images }
            });
            let worker_waiting = if view.task.checked { None } else { current.and_then(|run| projection::worker_waiting(state, run)) };
            let disposition = state.task_ownership.iter().find(|owner| owner.root_id == root_id && owner.task_id == task_id).and_then(|owner| owner.disposition.clone());
            tasks.push(InboxTaskLink { root_id: root_id.into(), task_id: task_id.into(), title: view.task.title.clone(), project: task_projects.get(&(root_id, task_id)).cloned().flatten(), lane, run, worker_waiting, disposition, evidence: task_evidence(state, root_id, task_id, current) });
        }
        let unread = latest_report.filter(|(m, _)| m.report.as_ref().is_some_and(|r| r.kind == ReportKind::Result))
            .filter(|(m, _)| !super::threads::is_read(state, &request.session_id, id, m))
            .map(|(m, _)| m.message_id.clone());
        let unread_interims: Vec<_> = messages.iter().copied().filter(|m|
            super::threads::current_worker_interim(state, &request.session_id, id, m)
                && !super::threads::is_read(state, &request.session_id, id, m))
            .filter_map(|m| interim_ref(state, m)).collect();
        let unresolved = projected_questions.iter().any(|q| matches!(q.receipt, QuestionReceipt::Unresolved));
        let answered_legacy = questions.get(id).is_some_and(|qs| qs.iter().any(|q| q.thread_id.is_none() && !matches!(q.receipt, QuestionReceipt::Unresolved))) && latest_report.is_some_and(|(m, _)| m.message_id == id);
        // The newest close covers only events present when it was written.
        // Positions are stable across archive compaction and hydration.
        let close = state.thread_reads.iter().rev().find_map(|read| {
            let (session, thread, message_fence, widget_fence) = super::threads::parse_close_marker(read)?;
            (session == request.session_id && thread == id).then_some((read, message_fence, widget_fence))
        });
        let close_coverage = close.map(|(record, message_fence, widget_fence)| {
            if message_fence > state.messages.len() || widget_fence > state.widget_publications.len()
                || messages.iter().any(|message| message_positions[&(*message as *const Message)] >= message_fence)
                || state.widget_publications.iter().enumerate().any(|(index, publication)|
                    index >= widget_fence && publication.thread_id == id && root_map.contains_key(publication.root_id.as_str()))
                || state.messages.iter().enumerate().skip(message_fence).any(|(_, answer)|
                    !answer.stale && answer.kind == MessageKind::Answer
                    && (matches!(answer.from, ActorRef::Operator) || matches!(&answer.from, ActorRef::Run { run_id }
                        if run_id == &answer.to_run_id && root_map.contains_key(run_id.as_str())))
                    && messages.iter().any(|question| report_owner(question, &root_map).is_some_and(|root|
                        root.run_id == answer.to_run_id && question.report.as_ref().is_some_and(|report|
                            report.kind == ReportKind::NeedsInput && answer.in_reply_to.as_deref() == Some(question.message_id.as_str())))))
            { return (record, false); }
            (record, true)
        });
        let reopened = close_coverage.is_some_and(|(_, covers)| !covers);
        let handled_result = latest_report.is_some_and(|(message, _)| {
            if !message.report.as_ref().is_some_and(|report| report.kind == ReportKind::Result) { return false; }
            if !reopened { return true; }
            let position = message_positions[&(message as *const Message)];
            !messages.iter().any(|later| message_positions[&(*later as *const Message)] > position)
                && !state.widget_publications.iter().any(|publication| publication.thread_id == id
                    && root_map.contains_key(publication.root_id.as_str()) && publication.message_position > position)
        });
        let (state_value, need) = if unresolved { (InboxState::NeedsYou, Some(InboxNeed::Decision)) }
            else if unread.is_some() || !unread_interims.is_empty() { (InboxState::NeedsYou, Some(InboxNeed::Answer)) }
            else if handled_result || (!reopened && ((link_count > 0 && tasks.len() == link_count && tasks.iter().all(|t| t.lane == TaskLane::Accepted)) || answered_legacy)) { (InboxState::Handled, None) }
            else { (InboxState::Underway, None) };
        let closed_at = if state_value == InboxState::Underway {
            close_coverage.filter(|(_, covers)| *covers).map(|(record, _)| record.at.clone())
        } else { None };
        let state_value = if closed_at.is_some() { InboxState::Handled } else { state_value };
        let linked_title = state.thread_links.iter().filter(|link| link.thread_id == id)
            .filter_map(|link| tasks.iter().find(|task| task.root_id == link.root_id && task.task_id == link.task_id).map(|task| (link, task)))
            .min_by(|(a, _), (b, _)| (&a.at, &a.root_id, &a.task_id).cmp(&(&b.at, &b.root_id, &b.task_id))).map(|(_, task)| task)
            .or_else(|| messages.iter().find_map(|m| {
                let task_id = m.handoff.as_ref()?.task_id.as_deref()?;
                tasks.iter().find(|task| task.root_id == m.to_run_id && task.task_id == task_id)
            }));
        let title = if let Some((m, root)) = latest_card { text(m.report.as_ref().unwrap().card.as_ref().unwrap().title.clone(), owner_source(root, true), Some(m)) }
            else if let Some(task) = linked_title {
                let worker_only = messages.iter().all(|message|
                    super::threads::linked_worker_interim(state, &request.session_id, id, message));
                let source = if worker_only { InboxSource::Recorded }
                    else { root_map.get(task.root_id.as_str()).map_or(InboxSource::Recorded, |root| owner_source(root, false)) };
                text(task.title.clone(), source, None)
            }
            else if let Some(m) = request_message { let line = request_title(&m.text); text(if line.trim().is_empty() && !m.images.is_empty() { format!("Image request ({})", m.images.len()) } else { line }, InboxSource::User, Some(m)) }
            else if let Some(q) = projected_questions.first() { text(first_line(&q.text.text), q.text.source.clone(), messages.iter().copied().find(|m| m.message_id == q.question_message_id && matches!(&m.from, ActorRef::Run { run_id } if run_id == &q.run_id))) }
            else if let Some((m, root)) = latest_report { text(first_line(&m.report.as_ref().unwrap().summary), owner_source(root, false), Some(m)) }
            else { text(id.to_owned(), InboxSource::Recorded, None) };
        let next = if let Some((m, root)) = latest_card { Some(text(m.report.as_ref().unwrap().card.as_ref().unwrap().next.clone(), owner_source(root, true), Some(m))) }
            else if let Some(receipt) = receipts.last() { let label = root_map.get(receipt.to_run_id.as_str()).map_or(receipt.to_run_id.as_str(), |r| r.label.as_str()); Some(text(if receipt.stage == DeliveryStage::Stored { format!("Waiting for {label} to pick up") } else { format!("With {label}") }, InboxSource::Recorded, None)) }
            else if tasks.len() == 1 { Some(text(format!("{} · {}", inbox_task_label(&tasks[0]), tasks[0].title), InboxSource::Recorded, None)) } else { None };
        // A question's recommendation belongs to that decision, not to the
        // thread forever. Use the same owner-scoped current-question receipts
        // as the decision UI, including answers still awaiting delivery.
        // Other reports can carry recommendations too; retain only the most
        // recent report for that owner, unless a later handoff superseded it.
        let recommendation = reports.iter().filter_map(|(m, root)| {
            let report = m.report.as_ref()?;
            let value = report.card.as_ref()?.recommendation.as_ref()?;
            let current = if report.kind == ReportKind::NeedsInput {
                projected_questions.iter().any(|q| q.run_id == root.run_id
                    && q.question_message_id == report.message_id
                    && matches!(q.receipt, QuestionReceipt::Unresolved))
            } else {
                let position = message_positions[&(*m as *const Message)];
                !messages.iter().any(|later| message_positions[&(*later as *const Message)] > position
                    && (report_owner(later, &root_map).is_some_and(|owner| owner.run_id == root.run_id)
                        || (later.to_run_id == root.run_id && !later.stale
                            && matches!(later.from, ActorRef::Operator)
                            && later.kind == MessageKind::Instruction && later.handoff.is_some())))
            };
            current.then_some((*m, *root, value))
        }).max_by_key(|(m, _, _)| message_positions[&(*m as *const Message)])
            .map(|(m, root, value)| text(value.clone(), owner_source(root, true), Some(m)));
        let reply = reports.iter().rev().find(|(m, _)| m.report.as_ref().is_some_and(|r| matches!(r.kind, ReportKind::Progress | ReportKind::Result))).map(|(m, root)| message_text(m.report.as_ref().unwrap().summary.clone(), owner_source(root, false), m));
        let project = latest_card.and_then(|(m, _)| m.report.as_ref()?.card.as_ref()?.project.clone());
        let mut facts = Vec::new();
        for r in &receipts { facts.push(text(format!("Delivery {:?}", r.stage), InboxSource::Recorded, messages.iter().copied().find(|m| m.message_id == r.message_id && m.to_run_id == r.to_run_id && matches!(m.from, ActorRef::Operator)))); }
        for t in &tasks {
            facts.push(text(format!("{} · {}", inbox_task_label(t), t.title), InboxSource::Recorded, None));
            if let Some(run) = &t.run {
                for item in &run.cleanup { if matches!(item.cleanup.state, CleanupState::Removed | CleanupState::Kept) { facts.push(text(format!("Cleanup {:?}: {}", item.cleanup.state, item.cleanup.reason.as_ref().map_or("", |r| r.message.as_str())), InboxSource::Recorded, None)); } }
                if let Some(worker) = state.runs.iter().find(|r| r.run_id == run.run_id) {
                    if let Some(fact) = integration_fact(worker) { facts.push(fact); }
                    if let Some(result) = &worker.result { facts.push(text(result.summary.clone(), InboxSource::Worker { run_id: worker.run_id.clone() }, None)); }
                    if let Some(output) = &worker.source_output {
                        let checks = match output { SourceOutput::Code { focused_checks, .. } | SourceOutput::NonCode { focused_checks, .. } => focused_checks };
                        for check in checks { facts.push(text(format!("{}: {:?}", check.command_or_scenario, check.outcome), InboxSource::Worker { run_id: worker.run_id.clone() }, None)); }
                    }
                }
            }
        }
        let updated_at = messages.iter().map(|m| m.created_at.as_str())
            .chain(state.widget_publications.iter().filter(|p| p.thread_id == id && root_map.contains_key(p.root_id.as_str())).map(|p| p.at.as_str()))
            .max().unwrap_or(now).to_owned();
        let (timeline, timeline_omitted, earlier_widgets, earlier_widgets_omitted) = timeline(&messages, state, id, &root_map, &boards);
        threads.push(InboxThread { thread_id: id.into(), state: state_value, need, closed_at, owner_root_ids: owner_ids.into_iter().map(str::to_owned).collect(), title, next, project, request: request_echo, follow_ups, questions: projected_questions, recommendation, reply, unread_result_message_id: unread, unread_interims, tasks, receipts, facts, updated_at, timeline, timeline_omitted, earlier_widgets: (!earlier_widgets.is_empty()).then_some(earlier_widgets), earlier_widgets_omitted });
    }
    threads.sort_by(|a,b| {
        let rank = |t: &InboxThread| match t.need { Some(InboxNeed::Decision) => 0, Some(InboxNeed::Answer) => 1, None if t.state == InboxState::Underway => 2, None => 3 };
        rank(a).cmp(&rank(b)).then_with(|| if rank(a) == 0 { a.questions.iter().filter(|q| matches!(q.receipt, QuestionReceipt::Unresolved)).map(|q| &q.asked_at).min().cmp(&b.questions.iter().filter(|q| matches!(q.receipt, QuestionReceipt::Unresolved)).map(|q| &q.asked_at).min()) } else if rank(a) == 1 {
            let result_time = |t: &InboxThread| t.unread_result_message_id.as_deref().and_then(|id| state.messages.iter().find(|m| m.message_id == id)).map(|m| m.created_at.as_str())
                .or_else(|| t.unread_interims.first().and_then(|item| state.messages.iter().find(|m| m.message_id == item.message_id && matches!(&m.from, ActorRef::Run { run_id } if run_id == &item.run_id))).map(|m| m.created_at.as_str()));
            result_time(a).cmp(&result_time(b))
        } else { b.updated_at.cmp(&a.updated_at) }).then_with(|| a.thread_id.cmp(&b.thread_id))
    });
    let mut handled = 0;
    if cap_handled {
        threads.retain(|t| { if t.state != InboxState::Handled { true } else { handled += 1; handled <= 50 } });
    }
    let observation = match observation { RuntimeObservation::Fresh { observed_at, .. } => InboxObservation::Fresh { observed_at }, RuntimeObservation::Unavailable { error } => InboxObservation::Unavailable { error } };
    let stalls = roots.iter().filter_map(|root| {
        let subtree: HashSet<&str> = runs.iter().filter(|run| run.root_id == root.run_id).map(|run| run.run_id.as_str()).collect();
        let prefix = format!("stalled:{}:", root.run_id);
        let stall = state.messages.iter().filter(|message| message.to_run_id == root.run_id
            && matches!(message.from, ActorRef::Dispatcher) && message.kind == MessageKind::Observation
            && message.message_id.starts_with(&prefix))
            .max_by(|a, b| message_order(a).cmp(&message_order(b)))?;
        let newer_run = runs.iter().any(|run| run.root_id == root.run_id && run.updated_at > stall.created_at);
        let newer_message = state.messages.iter().any(|message| message.created_at > stall.created_at
            && (subtree.contains(message.to_run_id.as_str())
                || matches!(&message.from, ActorRef::Run { run_id } if subtree.contains(run_id.as_str()))));
        (!newer_run && !newer_message).then(|| InboxStall {
            root_id: root.run_id.clone(), since: stall.created_at.clone(), text: stall.text.clone(),
        })
    }).collect();
    Ok(InboxView { session_id: request.session_id.clone(), state_store_id: store_id.into(), revision: state.revision, tasks_token: String::new(), observation, owners, threads, unmanaged_agents, stalls, image_limits: ImageLimits { max_bytes: super::images::MAX_IMAGE_BYTES as _, max_count: super::images::MAX_MESSAGE_IMAGES as _, mime_types: vec!["image/png".into(), "image/jpeg".into()] } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::store::{ThreadLinkRecord, ThreadReadRecord};

    const NOW: &str = "2026-10-05T12:00:00Z";
    fn root(id: &str) -> Run {
        Run { session_id: "session".into(), brief: String::new(), run_id: id.into(),
            kind: RunKind::Supervisor, label: id.into(), root_id: id.into(), parent_run_id: None,
            task_id: None, attempt: 1, assignment_generation: None,
            launch_config: Default::default(), task_revision_at_start: None, stage: RunStage::Active,
            close_reason: None, dispatch: None, target: None, setup: None, cleanup_policy: None,
            last_report: None, result: None, source_output: None, integration: Integration::Pending,
            annotations: vec![], project: None, checkout: None, attachment: None, checkout_claim_id: None,
            resume: None, retirement: None, created_at: NOW.into(), updated_at: NOW.into() }
    }
    #[test]
    fn integration_facts_distinguish_source_from_recorded_outcomes() {
        let mut run = root("worker");
        assert!(integration_fact(&run).is_none());
        let code = SourceOutput::Code { repository_key: "repo".into(), checkout_key: "checkout".into(),
            base_commit: "base".into(), head_commit: "head".into(), commits: vec![],
            task_revision: "rev".into(), result_message_id: "result".into(), focused_checks: vec![] };
        run.source_output = Some(code.clone());
        assert!(integration_fact(&run).is_none());
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Accepted);
        assert!(integration_fact(&run).is_none());
        let mut result = report("result", "worker", 1, None, ReportKind::Result).report.unwrap();
        result.outcome = Some(ReportOutcome::Succeeded);
        result.source_output = Some(code.clone());
        run.result = Some(result);
        let fact = integration_fact(&run).unwrap();
        assert_eq!(fact.text, "Integration: Pending");
        assert!(matches!(fact.source, InboxSource::Recorded));
        run.source_output = Some(SourceOutput::NonCode { result_message_id: "result".into(),
            task_revision: "rev".into(), artifacts: vec![], focused_checks: vec![] });
        let fact = integration_fact(&run).unwrap();
        assert_eq!(fact.text, "Git integration not needed for non-code output");
        assert!(matches!(fact.source, InboxSource::Worker { ref run_id } if run_id == "worker"));
        assert!(matches!(run.integration, Integration::Pending));
        run.integration = Integration::NotApplicable { result_message_id: "result".into(), reason: "accepted".into() };
        let fact = integration_fact(&run).unwrap();
        assert_eq!(fact.text, "Integration: Not applicable");
        assert!(matches!(fact.source, InboxSource::Recorded));
        run.integration = Integration::Integrated { operation_id: "op".into(), accepted_result_message_id: "result".into(),
            accepted_task_revision: "rev".into(), source: Box::new(code), target_repository_key: "repo".into(),
            target_ref: "refs/heads/main".into(), target_commit: "head".into(), method: IntegrationMethod::Ancestor,
            mappings: Box::new([]), checks: vec![], integrator_run_id: "integrator".into(), verified_at: NOW.into() };
        let fact = integration_fact(&run).unwrap();
        assert_eq!(fact.text, "Integration: Integrated");
        assert!(matches!(fact.source, InboxSource::Recorded));
    }
    fn message(id: &str, root: &str, seq: u64, thread: Option<&str>, kind: MessageKind) -> Message {
        Message { message_id: id.into(), to_run_id: root.into(), seq, from: ActorRef::Operator,
            kind, text: id.into(), in_reply_to: None, report: None, thread_id: thread.map(str::to_owned),
            handoff: None, stale: false, escalated_from: None, stage: DeliveryStage::Stored,
            created_at: format!("2026-10-05T12:{:02}:{:02}Z", seq / 60, seq % 60), acked_at: None, images: vec![] }
    }
    fn report(id: &str, root: &str, seq: u64, thread: Option<&str>, kind: ReportKind) -> Message {
        let mut m = message(id, root, seq, thread, MessageKind::Report);
        m.from = ActorRef::Run { run_id: root.into() };
        m.report = Some(Report { message_id: id.into(), kind, outcome: None, summary: format!("summary {id}"),
            plan: None, source_output: None, card: None, at: m.created_at.clone() });
        m
    }
    fn image(id: &str) -> MessageImage {
        MessageImage { image_id: id.into(), mime_type: "image/png".into(), width: 1, height: 1, bytes: 42 }
    }

    fn recommend(message: &mut Message, value: &str) {
        message.report.as_mut().unwrap().card = Some(ThreadCard {
            title: "Title".into(), next: "Next".into(), recommendation: Some(value.into()),
            choices: vec![], project: None,
        });
    }

    #[test]
    fn recommendation_tracks_question_receipt_and_result() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut question = report("q", "r", 1, Some("t"), ReportKind::NeedsInput);
        recommend(&mut question, "choose A");
        state.messages.push(question);
        assert_eq!(view(&state, vec![]).threads[0].recommendation.as_ref().unwrap().text, "choose A");
        let mut answer = message("answer", "r", 2, Some("t"), MessageKind::Answer);
        answer.in_reply_to = Some("q".into());
        state.messages.push(answer);
        for stage in [DeliveryStage::Stored, DeliveryStage::Read, DeliveryStage::Acked] {
            state.messages.last_mut().unwrap().stage = stage;
            let projected = view(&state, vec![]);
            let thread = &projected.threads[0];
            assert!(thread.recommendation.is_none());
            assert_eq!(thread.timeline.last().unwrap().text, "answer");
            assert!(!matches!(thread.questions[0].receipt, QuestionReceipt::Unresolved));
        }
        state.messages.push(report("result", "r", 3, Some("t"), ReportKind::Result));
        let projected = view(&state, vec![]);
        assert!(projected.threads[0].recommendation.is_none());
        assert_eq!(projected.threads[0].reply.as_ref().unwrap().text, "summary result");
    }

    #[test]
    fn recommendation_uses_current_owner_question_not_another_owners_answer() {
        let mut state = OrchestrationState { runs: vec![root("a"), root("b")], ..Default::default() };
        let mut a = report("shared", "a", 1, Some("t"), ReportKind::NeedsInput);
        recommend(&mut a, "A");
        let mut b = report("shared", "b", 2, Some("t"), ReportKind::NeedsInput);
        recommend(&mut b, "B");
        let mut answer = message("answer", "b", 3, Some("t"), MessageKind::Answer);
        answer.in_reply_to = Some("shared".into());
        state.messages.extend([a, b, answer]);
        let projected = view(&state, vec![]);
        let thread = &projected.threads[0];
        assert_eq!(thread.recommendation.as_ref().unwrap().text, "A");
        assert_eq!(thread.need, Some(InboxNeed::Decision));
        let mut replacement = report("replacement", "a", 4, Some("t"), ReportKind::NeedsInput);
        recommend(&mut replacement, "new A");
        state.messages.push(replacement);
        assert_eq!(view(&state, vec![]).threads[0].recommendation.as_ref().unwrap().text, "new A");
        state.messages.push(report("result", "a", 5, Some("t"), ReportKind::Result));
        assert!(view(&state, vec![]).threads[0].recommendation.is_none());
    }

    #[test]
    fn recommendation_preserves_current_non_question_and_open_followup_question() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut initial = report("initial", "r", 0, Some("t"), ReportKind::Progress);
        recommend(&mut initial, "progress advice");
        state.messages.push(initial);
        assert_eq!(view(&state, vec![]).threads[0].recommendation.as_ref().unwrap().text, "progress advice");
        let mut result = report("result", "r", 1, Some("t"), ReportKind::Result);
        recommend(&mut result, "next action");
        state.messages.push(result);
        assert_eq!(view(&state, vec![]).threads[0].recommendation.as_ref().unwrap().text, "next action");
        let mut follow = message("follow", "r", 2, Some("t"), MessageKind::Instruction);
        follow.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        state.messages.push(follow);
        assert!(view(&state, vec![]).threads[0].recommendation.is_none());
        let mut question = report("q", "r", 3, Some("t"), ReportKind::NeedsInput);
        recommend(&mut question, "decision");
        state.messages.push(question);
        let mut unrelated = message("other-answer", "r", 4, Some("t"), MessageKind::Answer);
        unrelated.in_reply_to = Some("other".into());
        state.messages.push(unrelated);
        let mut progress = report("progress", "r", 5, Some("t"), ReportKind::Progress);
        progress.stale = true;
        state.messages.push(progress);
        let projected = view(&state, vec![]);
        assert_eq!(projected.threads[0].recommendation.as_ref().unwrap().text, "decision");
        assert_eq!(projected.threads[0].need, Some(InboxNeed::Decision));
    }

    #[test]
    fn timeline_includes_root_self_answer_for_legacy_question_with_owner_source() {
        let mut state = OrchestrationState { runs: vec![root("r"), root("other")], ..Default::default() };
        let mut question = report("question", "r", 10, None, ReportKind::NeedsInput);
        question.created_at = "2026-10-06T12:00:00Z".into();
        let mut answer = message("self-answer", "r", 1, Some("question"), MessageKind::Answer);
        answer.from = ActorRef::Run { run_id: "r".into() };
        answer.in_reply_to = Some("question".into());
        answer.created_at = "2026-10-04T12:00:00Z".into();
        let mut other = message("other-answer", "other", 2, Some("question"), MessageKind::Answer);
        other.from = ActorRef::Run { run_id: "other".into() };
        other.in_reply_to = Some("question".into());
        let mut wrong_id = message("wrong-id", "r", 3, Some("question"), MessageKind::Answer);
        wrong_id.from = ActorRef::Run { run_id: "r".into() };
        wrong_id.in_reply_to = Some("another-question".into());
        let mut stale = message("stale", "r", 4, Some("question"), MessageKind::Answer);
        stale.from = ActorRef::Run { run_id: "r".into() };
        stale.in_reply_to = Some("question".into());
        stale.stale = true;
        state.messages.extend([question, answer, other, wrong_id, stale]);

        let projected = view(&state, vec![]);
        let thread = projected.threads.iter().find(|thread| thread.thread_id == "question").unwrap();
        assert_eq!(thread.timeline.iter().map(|entry| entry.message_id.as_str()).collect::<Vec<_>>(),
            vec!["question", "self-answer"]);
        let answer = &thread.timeline[1];
        assert_eq!(answer.kind, InboxEntryKind::Answer);
        assert!(matches!(&answer.source, InboxSource::Owner { run_id, .. } if run_id == "r"));
        assert_eq!(answer.in_reply_to.as_deref(), Some("question"));
    }

    #[test]
    fn projected_questions_follow_timeline_order_across_owners() {
        let mut state = OrchestrationState { runs: vec![root("a"), root("b")], ..Default::default() };
        let mut first = report("shared", "b", 90, Some("t"), ReportKind::NeedsInput);
        first.created_at = "2026-10-06T12:00:00Z".into();
        first.report.as_mut().unwrap().at = first.created_at.clone();
        let mut second = report("shared", "a", 1, Some("t"), ReportKind::NeedsInput);
        second.created_at = "2026-10-04T12:00:00Z".into();
        second.report.as_mut().unwrap().at = second.created_at.clone();
        state.messages.extend([first, second]);

        let projected = view(&state, vec![]);
        let thread = &projected.threads[0];
        assert_eq!(thread.questions.iter().map(|q| q.run_id.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
        assert_eq!(thread.timeline.iter().map(|entry| match &entry.source {
            InboxSource::Owner { run_id, .. } => run_id.as_str(),
            _ => panic!("expected owner question"),
        }).collect::<Vec<_>>(), vec!["b", "a"]);
    }

    #[test]
    fn task_decisions_follow_persisted_order_and_latest_matching_answer() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.task_id = Some("task".into());
        let mut other = worker.clone();
        other.run_id = "other".into();
        let mut state = OrchestrationState { runs: vec![root("r"), worker, other], ..Default::default() };
        let mut first = report("q1", "worker", 90, None, ReportKind::NeedsInput);
        first.created_at = "2026-10-06T12:00:00Z".into();
        first.report.as_mut().unwrap().message_id = "report-q1".into();
        let mut second = report("q2", "other", 1, None, ReportKind::NeedsInput);
        second.created_at = "2026-10-04T12:00:00Z".into();
        let mut original = message("original", "worker", 1, None, MessageKind::Answer);
        original.in_reply_to = Some("report-q1".into());
        original.created_at = "2026-10-07T12:00:00Z".into();
        let mut correction = message("correction", "worker", 100, None, MessageKind::Answer);
        correction.in_reply_to = Some("report-q1".into());
        correction.created_at = "2026-10-03T12:00:00Z".into();
        correction.from = ActorRef::Run { run_id: "r".into() };
        let mut later_lower_seq = message("later-lower-seq", "worker", 2, None, MessageKind::Answer);
        later_lower_seq.in_reply_to = Some("report-q1".into());
        let mut wrong_recipient = message("wrong", "other", 200, None, MessageKind::Answer);
        wrong_recipient.in_reply_to = Some("report-q1".into());
        let mut wrong_id = message("wrong-id", "worker", 300, None, MessageKind::Answer);
        wrong_id.in_reply_to = Some("q1".into());
        let mut stale = message("stale", "worker", 400, None, MessageKind::Answer);
        stale.in_reply_to = Some("report-q1".into());
        stale.stale = true;
        state.messages.extend([first, original, second, correction, later_lower_seq, wrong_recipient, wrong_id, stale]);

        let decisions = task_evidence(&state, "r", "task", Some(&state.runs[1])).unwrap().decisions;
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions.iter().map(|d| d.question.as_str()).collect::<Vec<_>>(), vec!["summary q1", "summary q2"]);
        assert_eq!(decisions[0].answer.as_deref(), Some("correction"));
        assert_eq!(decisions[0].answered_at.as_deref(), Some("2026-10-03T12:00:00Z"));
        assert!(matches!(decisions[0].by, Some(InboxDecisionBy::Owner)));
        assert!(decisions[1].answer.is_none());
        assert!(decisions[1].answered_at.is_none());
        assert!(decisions[1].by.is_none());
    }

    #[test]
    fn task_decisions_keep_latest_twenty_in_persisted_order() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.task_id = Some("task".into());
        let mut other = worker.clone();
        other.run_id = "other".into();
        let mut state = OrchestrationState { runs: vec![root("r"), worker, other], ..Default::default() };
        for i in 0..24 {
            let mut question = report(&format!("q{i}"), if i % 2 == 0 { "worker" } else { "other" },
                (24 - i) as u64, None, ReportKind::NeedsInput);
            question.created_at = format!("2026-10-{:02}T12:00:00Z", 25 - i);
            state.messages.push(question);
        }
        let mut stale = report("stale", "worker", 1, None, ReportKind::NeedsInput);
        stale.stale = true;
        state.messages.push(stale);
        state.messages.push(report("outside", "r", 1, None, ReportKind::NeedsInput));

        let decisions = task_evidence(&state, "r", "task", Some(&state.runs[1])).unwrap().decisions;
        assert_eq!(decisions.iter().map(|d| d.question.clone()).collect::<Vec<_>>(),
            (4..24).map(|i| format!("summary q{i}")).collect::<Vec<_>>());
    }
    #[test]
    fn worker_directive_is_a_user_timeline_entry_with_task_title() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.parent_run_id = Some("r".into());
        worker.task_id = Some("task".into());
        let mut state = OrchestrationState { runs: vec![root("r"), worker], ..Default::default() };
        let mut request = message("request", "r", 1, Some("t"), MessageKind::Instruction);
        request.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        let mut directive = message("directive", "worker", 2, Some("t"), MessageKind::UserDirective);
        directive.handoff = Some(HandoffRecord { task_id: Some("task".into()), task_revision: None, attempt_run_id: Some("worker".into()), binding: None, origin: OperatorOrigin::Browser });
        state.messages.extend([request, directive]);
        let result = view(&state, vec![board("r", "task")]);
        let thread = &result.threads[0];
        assert_eq!(thread.owner_root_ids, vec!["r"]);
        assert!(thread.follow_ups.is_empty());
        let entry = thread.timeline.iter().find(|entry| entry.message_id == "directive").unwrap();
        assert_eq!(entry.kind, InboxEntryKind::Directive);
        assert!(matches!(entry.source, InboxSource::User));
        assert_eq!(entry.task_title.as_deref(), Some("Task"));
        assert_eq!(thread.tasks.len(), 1);
    }

    #[test]
    fn stall_disappears_after_subtree_activity() {
        let mut worker = root("worker");
        worker.root_id = "r".into();
        worker.parent_run_id = Some("r".into());
        worker.kind = RunKind::Worker;
        let mut state = OrchestrationState { runs: vec![root("r"), worker], ..Default::default() };
        let mut stall = message("stalled:r:key", "r", 2, None, MessageKind::Observation);
        stall.from = ActorRef::Dispatcher;
        state.messages.push(stall);
        let projected = view(&state, vec![]);
        assert_eq!(projected.stalls.len(), 1);
        assert_eq!(projected.stalls[0].root_id, "r");
        let mut activity = message("activity", "r", 3, None, MessageKind::Observation);
        activity.from = ActorRef::Run { run_id: "worker".into() };
        state.messages.push(activity);
        assert!(view(&state, vec![]).stalls.is_empty());
    }
    #[test]
    fn image_projection_preserves_handoffs_reports_and_inert_markdown() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut first = message("first", "r", 1, Some("t"), MessageKind::Instruction);
        first.text = String::new();
        first.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        first.images = vec![image("first-image")];
        let mut follow = message("follow", "r", 2, Some("t"), MessageKind::Instruction);
        follow.handoff = first.handoff.clone();
        follow.images = vec![image("follow-image")];
        let mut reply = report("reply", "r", 3, Some("t"), ReportKind::Progress);
        reply.report.as_mut().unwrap().summary = "![x](https://example.invalid/image.png)".into();
        reply.images = vec![image("reply-image")];
        let mut question = report("question", "r", 4, Some("t"), ReportKind::NeedsInput);
        question.images = vec![image("question-image")];
        state.messages.extend([follow, question, first, reply]);
        let result = view(&state, vec![]);
        let thread = &result.threads[0];
        assert_eq!(thread.title.text, "Image request (1)");
        assert!(thread.title.images.is_empty());
        assert_eq!(thread.request.as_ref().unwrap().images, vec![image("first-image")]);
        assert_eq!(thread.follow_ups.len(), 1);
        assert_eq!(thread.follow_ups[0].message_id, "follow");
        assert_eq!(thread.follow_ups[0].images, vec![image("follow-image")]);
        assert_eq!(thread.reply.as_ref().unwrap().text, "![x](https://example.invalid/image.png)");
        assert_eq!(thread.reply.as_ref().unwrap().images, vec![image("reply-image")]);
        assert_eq!(thread.questions[0].text.images, vec![image("question-image")]);
        assert!(thread.facts.iter().all(|fact| fact.images.is_empty()));
        assert_eq!(result.image_limits.max_bytes, super::super::images::MAX_IMAGE_BYTES);
        assert_eq!(result.image_limits.max_count, super::super::images::MAX_MESSAGE_IMAGES as u32);
        assert_eq!(result.image_limits.mime_types, vec!["image/png", "image/jpeg"]);
    }
    #[test]
    fn inbox_projects_only_the_linked_tasks_disposition() {
        let mut state = OrchestrationState { runs: vec![root("r"), root("other")], ..Default::default() };
        let disposition = TaskDisposition {
            disposition_id: "disposition".into(), kind: TaskDispositionKind::Abandoned, reason: "No longer needed".into(), evidence: Some("Decision".into()),
            successor_task_id: None, successor_run_id: None, successor_task_revision: None, successor_result_message_id: None, successor_accepted_task_revision: None,
            at: NOW.into(), by: ActorRef::Operator, reopened_at: None, reopened_by: None,
        };
        state.task_ownership.push(TaskOwnership { root_id: "other".into(), task_id: "task".into(), generation: 1, disposition: Some(disposition.clone()), disposition_history: Vec::new(), owner: None, schedule: None, reopened_after_attempt: None });
        state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: "task".into(), generation: 1, disposition: None, disposition_history: Vec::new(), owner: None, schedule: None, reopened_after_attempt: None });
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        state.messages.push(report("root-progress", "r", 1, Some("t"), ReportKind::Progress));
        assert!(view(&state, vec![board("r", "task")]).threads[0].tasks[0].disposition.is_none());
        state.task_ownership[1].disposition = Some(disposition);
        let projected = view(&state, vec![board("r", "task")]);
        let actual = projected.threads[0].tasks[0].disposition.as_ref().unwrap();
        assert_eq!(actual.kind, TaskDispositionKind::Abandoned);
        assert_eq!(actual.reason, "No longer needed");
        assert_eq!(actual.evidence.as_deref(), Some("Decision"));
        state.task_ownership[1].disposition = None;
        assert!(view(&state, vec![board("r", "task")]).threads[0].tasks[0].disposition.is_none());
    }
    #[test]
    fn worker_images_use_latest_non_stale_image_report() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.parent_run_id = Some("r".into());
        worker.task_id = Some("task".into());
        worker.assignment_generation = Some(1);
        worker.attachment = Some(ExecutionAttachment {
            attachment_id: "worker-attachment".into(), epoch: 3, state: AttachmentState::Connected,
            launch_mode: None, location: None, conversation: None, process: None, shell: None,
            machine_id: None, process_scope: None, launch_attempt: 1, updated_at: NOW.into(),
        });
        let mut state = OrchestrationState { runs: vec![root("r"), worker], ..Default::default() };
        state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: "task".into(), generation: 1, disposition: None, disposition_history: Vec::new(), schedule: None, reopened_after_attempt: None,
            owner: Some(TaskOwner { run_id: "worker".into(), conversation: None, claim_operation_id: "claim".into() }) });
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        state.messages.push(report("root-progress", "r", 1, Some("t"), ReportKind::Progress));
        let mut old = report("old", "worker", 2, None, ReportKind::Progress);
        old.images = vec![image("old")];
        let mut newest = report("newest", "worker", 3, None, ReportKind::Progress);
        newest.images = vec![image("newest")];
        let empty = report("empty", "worker", 4, None, ReportKind::Progress);
        let mut stale = report("stale", "worker", 5, None, ReportKind::Result);
        stale.stale = true;
        stale.images = vec![image("stale")];
        state.messages.extend([stale, newest, old, empty]);
        let result = view(&state, vec![board("r", "task")]);
        let owner = &result.owners[0];
        assert_eq!((&*owner.run_id, owner.attempt, &*owner.session_id), ("r", 1, "session"));
        assert!(owner.attachment_id.is_none());
        assert!(owner.attachment_epoch.is_none());
        let run = result.threads[0].tasks[0].run.as_ref().unwrap();
        assert_eq!((&*run.run_id, run.attempt, &*run.session_id), ("worker", 1, "session"));
        assert_eq!(run.attachment_id.as_deref(), Some("worker-attachment"));
        assert_eq!(run.attachment_epoch, Some(3));
        assert_eq!(result.threads[0].tasks[0].run.as_ref().unwrap().report_images, vec![image("newest")]);
    }
    #[test]
    fn inbox_task_exposes_worker_wait_independently_of_thread_question() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.parent_run_id = Some("r".into());
        worker.task_id = Some("task".into());
        worker.assignment_generation = Some(1);
        worker.stage = RunStage::Working;
        let mut state = OrchestrationState { runs: vec![root("r"), worker], ..Default::default() };
        state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: "task".into(), generation: 1, disposition: None, disposition_history: Vec::new(),
            schedule: None, reopened_after_attempt: None, owner: Some(TaskOwner { run_id: "worker".into(),
                conversation: None, claim_operation_id: "claim".into() }) });
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(),
            task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        let question = report("question", "r", 1, Some("t"), ReportKind::NeedsInput);
        let mut waiting = report("wait", "worker", 2, None, ReportKind::Progress);
        waiting.to_run_id = "r".into();
        let wait_report = waiting.report.as_mut().unwrap();
        wait_report.summary = "Waiting for review".into();
        wait_report.plan = Some(WorkerCheckpoint { kind: WorkerCheckpointKind::Waiting, artifacts: vec![] }.encode());
        state.messages.extend([question, waiting]);
        let mut tasks = board("r", "task");
        tasks.tasks[0].task.checked = false;
        let projected = view(&state, vec![tasks.clone()]);
        assert_eq!(projected.threads[0].need, Some(InboxNeed::Decision));
        let wait = projected.threads[0].tasks[0].worker_waiting.as_ref().unwrap();
        assert_eq!(wait.cause, TaskWaitCause::WorkerWaiting);
        assert_eq!(wait.detail.as_deref(), Some("Waiting for review"));
        let mut resumed = report("resumed", "worker", 3, None, ReportKind::Progress);
        resumed.report.as_mut().unwrap().plan = Some(WorkerCheckpoint { kind: WorkerCheckpointKind::Resumed, artifacts: vec![] }.encode());
        state.messages.push(resumed);
        assert!(view(&state, vec![tasks]).threads[0].tasks[0].worker_waiting.is_none());
    }
    #[test]
    fn linked_interims_preserve_history_and_independent_exact_unread_identity() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.parent_run_id = Some("r".into());
        worker.task_id = Some("task".into());
        worker.assignment_generation = Some(2);
        worker.stage = RunStage::Working;
        let mut old = worker.clone();
        old.run_id = "old".into();
        old.assignment_generation = Some(1);
        let mut unrelated = worker.clone();
        unrelated.run_id = "unrelated".into();
        unrelated.task_id = Some("other".into());
        let mut foreign = worker.clone();
        foreign.run_id = "foreign".into();
        foreign.root_id = "other-root".into();
        let mut state = OrchestrationState { runs: vec![root("r"), root("other-root"), worker, old, unrelated, foreign], ..Default::default() };
        state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: "task".into(), generation: 2, disposition: None, disposition_history: Vec::new(),
            schedule: None, reopened_after_attempt: None, owner: Some(TaskOwner { run_id: "worker".into(),
                conversation: None, claim_operation_id: "claim".into() }) });
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        let worker_report = |id, sender, seq| {
            let mut m = report(id, sender, seq, None, ReportKind::Progress);
            m.report.as_mut().unwrap().plan = Some(WorkerCheckpoint { kind: WorkerCheckpointKind::Interim, artifacts: vec![] }.encode());
            m.to_run_id = "r".into();
            m
        };
        let mut stale = worker_report("stale", "worker", 2);
        stale.stale = true;
        let mut one = worker_report("one", "worker", 3);
        one.report.as_mut().unwrap().plan = Some(WorkerCheckpoint { kind: WorkerCheckpointKind::Interim, artifacts: vec!["/tmp/checkpoint.md".into()] }.encode());
        let mut waiting = worker_report("waiting", "worker", 8);
        waiting.report.as_mut().unwrap().plan = Some(WorkerCheckpoint { kind: WorkerCheckpointKind::Waiting, artifacts: vec![] }.encode());
        state.messages.extend([worker_report("old", "old", 1), stale,
            one, worker_report("two", "worker", 4),
            worker_report("wrong-task", "unrelated", 5), worker_report("wrong-root", "foreign", 6),
            report("result", "r", 7, Some("t"), ReportKind::Result), waiting]);
        let mut worker_only = state.clone();
        worker_only.messages.retain(|message| message.message_id == "one");
        let worker_thread = view(&worker_only, vec![board("r", "task")]);
        assert!(matches!(worker_thread.threads[0].title.source, InboxSource::Recorded));
        let projected = view(&state, vec![board("r", "task")]);
        let thread = &projected.threads[0];
        assert_eq!(thread.unread_interims.iter().map(|item| item.message_id.as_str()).collect::<Vec<_>>(), vec!["one", "two"]);
        assert_eq!(thread.unread_result_message_id.as_deref(), Some("result"));
        assert_eq!(thread.state, InboxState::NeedsYou);
        assert_eq!(thread.timeline.iter().map(|e| e.message_id.as_str()).collect::<Vec<_>>(),
            vec!["old", "stale", "one", "two", "result"]);
        assert!(thread.timeline[..4].iter().all(|e| e.kind == InboxEntryKind::Interim && e.outcome.is_none()
            && matches!(&e.source, InboxSource::Worker { .. })));
        let one = &thread.timeline[2];
        let identity = one.interim.as_ref().unwrap();
        assert_eq!((&*identity.run_id, &*identity.task_id, identity.attempt), ("worker", "task", 1));
        assert_eq!(one.artifacts.iter().map(|artifact| artifact.path.as_str()).collect::<Vec<_>>(), vec!["/tmp/checkpoint.md"]);
        let two_key = super::super::threads::interim_read_key(&state, &state.messages[3]).unwrap();
        state.thread_reads.push(ThreadReadRecord { thread_id: "t".into(), through_message_id: two_key,
            origin: ActorRef::Operator, at: NOW.into() });
        let projected = view(&state, vec![board("r", "task")]);
        assert_eq!(projected.threads[0].unread_interims.iter().map(|item| item.message_id.as_str()).collect::<Vec<_>>(), vec!["one"]);
        assert_eq!(projected.threads[0].unread_result_message_id.as_deref(), Some("result"));
        let old_key = super::super::threads::interim_read_key(&state, &state.messages[0]).unwrap();
        state.thread_reads.push(ThreadReadRecord { thread_id: "t".into(), through_message_id: old_key,
            origin: ActorRef::Operator, at: NOW.into() });
        assert_eq!(view(&state, vec![board("r", "task")]).threads[0].unread_interims.len(), 1);
        let one_key = super::super::threads::interim_read_key(&state, &state.messages[2]).unwrap();
        state.thread_reads.last_mut().unwrap().through_message_id = one_key;
        assert!(view(&state, vec![board("r", "task")]).threads[0].unread_interims.is_empty());
        state.task_ownership[0].owner = None;
        assert!(view(&state, vec![board("r", "task")]).threads[0].unread_interims.is_empty());
    }

    #[test]
    fn legacy_result_read_does_not_acknowledge_same_id_worker_interim() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker; worker.root_id = "r".into(); worker.parent_run_id = Some("r".into());
        worker.task_id = Some("task".into()); worker.assignment_generation = Some(1); worker.stage = RunStage::Working;
        let mut state = OrchestrationState { runs: vec![root("r"), worker], ..Default::default() };
        state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: "task".into(), generation: 1, disposition: None, disposition_history: Vec::new(),
            schedule: None, reopened_after_attempt: None, owner: Some(TaskOwner { run_id: "worker".into(), conversation: None, claim_operation_id: "claim".into() }) });
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        let mut interim = report("same", "worker", 1, None, ReportKind::Progress);
        interim.report.as_mut().unwrap().plan = Some(WorkerCheckpoint { kind: WorkerCheckpointKind::Interim, artifacts: vec![] }.encode());
        interim.to_run_id = "r".into();
        state.messages.extend([interim, report("same", "r", 2, Some("t"), ReportKind::Result)]);
        state.thread_reads.push(ThreadReadRecord { thread_id: "t".into(), through_message_id: "same".into(),
            origin: ActorRef::Operator, at: NOW.into() });
        let projected = view(&state, vec![board("r", "task")]);
        assert_eq!(projected.threads[0].unread_interims[0].message_id, "same");
        assert!(projected.threads[0].unread_result_message_id.is_none());
        assert!(super::super::threads::read_target(&state, "session", "t", "same", Some("worker")).is_some());
    }

    #[test]
    fn interim_selection_is_scoped_by_sender_and_requires_authorized_link() {
        let mut first = root("first");
        first.kind = RunKind::Worker; first.root_id = "r".into(); first.parent_run_id = Some("r".into());
        first.task_id = Some("task".into()); first.assignment_generation = Some(1); first.stage = RunStage::Working;
        let mut second = first.clone();
        second.run_id = "second".into(); second.task_id = Some("task2".into());
        let mut state = OrchestrationState { runs: vec![root("r"), first, second], ..Default::default() };
        for (task, worker) in [("task", "first"), ("task2", "second")] {
            state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: task.into(), generation: 1, disposition: None, disposition_history: Vec::new(),
                schedule: None, reopened_after_attempt: None, owner: Some(TaskOwner { run_id: worker.into(), conversation: None, claim_operation_id: "claim".into() }) });
            state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: task.into(), linked_by: ActorRef::Operator, at: NOW.into() });
            let mut m = report("shared", worker, 1, None, ReportKind::Progress);
            m.to_run_id = "r".into();
            m.report.as_mut().unwrap().plan = Some(WorkerCheckpoint { kind: WorkerCheckpointKind::Interim, artifacts: vec![] }.encode());
            state.messages.push(m);
        }
        let projected = view(&state, vec![board("r", "task"), board("r", "task2")]);
        assert_eq!(projected.threads[0].unread_interims.len(), 2);
        assert!(super::super::threads::read_target(&state, "session", "t", "shared", None).is_none());
        let target = super::super::threads::read_target(&state, "session", "t", "shared", Some("first")).unwrap();
        let key = super::super::threads::interim_read_key(&state, target).unwrap();
        state.thread_reads.push(ThreadReadRecord { thread_id: "t".into(), through_message_id: key, origin: ActorRef::Operator, at: NOW.into() });
        let projected = view(&state, vec![board("r", "task"), board("r", "task2")]);
        assert_eq!(projected.threads[0].unread_interims.len(), 1);
        assert_eq!(projected.threads[0].unread_interims[0].run_id, "second");
        assert!(super::super::threads::read_target(&state, "session", "wrong-thread", "shared", Some("second")).is_none());
        assert!(super::super::threads::read_target(&state, "other-session", "t", "shared", Some("second")).is_none());
        state.messages[1].to_run_id = "first".into();
        assert!(super::super::threads::read_target(&state, "session", "t", "shared", Some("second")).is_none());
    }
    #[test]
    fn worker_claim_facts_keep_full_markdown_after_many_receipts() {
        let summary = format!("{}\n\n- **Evidence:** `cargo test`\n- [artifact](https://example.invalid/result)\n", "Long result ".repeat(12));
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.parent_run_id = Some("r".into());
        worker.task_id = Some("task".into());
        worker.assignment_generation = Some(1);
        worker.result = report("worker-result", "worker", 1, None, ReportKind::Result).report;
        worker.result.as_mut().unwrap().summary = summary.clone();
        let mut state = OrchestrationState { runs: vec![root("r"), worker], ..Default::default() };
        state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: "task".into(), generation: 1, disposition: None, disposition_history: Vec::new(), schedule: None, reopened_after_attempt: None,
            owner: Some(TaskOwner { run_id: "worker".into(), conversation: None, claim_operation_id: "claim".into() }) });
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        for seq in 1..=10 {
            state.messages.push(message(&format!("receipt-{seq}"), "r", seq, Some("t"), MessageKind::Instruction));
        }
        let projected = view(&state, vec![board("r", "task")]);
        let facts = &projected.threads[0].facts;
        assert!(facts.len() > 8);
        let claim = facts.iter().find(|fact| matches!(&fact.source, InboxSource::Worker { run_id } if run_id == "worker")).unwrap();
        assert_eq!(claim.text, summary);
        assert!(claim.message_id.is_none());
        assert!(claim.at.is_none());
        assert!(matches!(&facts[0].source, InboxSource::Recorded));
    }
    #[test]
    fn owner_report_title_remains_first_line_and_utf8_bounded() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut owner_report = report("progress", "r", 1, Some("t"), ReportKind::Progress);
        owner_report.report.as_mut().unwrap().summary = format!("{}é\nMore details", "a".repeat(79));
        state.messages.push(owner_report);
        let projected = view(&state, vec![]);
        assert_eq!(projected.threads[0].title.text, "a".repeat(79));
    }
    fn view(state: &OrchestrationState, boards: Vec<TaskBoard>) -> InboxView {
        project(&InboxRequest { session_id: "session".into() }, state, boards,
            Err(InspectionError::new("offline", "unavailable")), NOW, "store").unwrap()
    }
    fn board(root: &str, task: &str) -> TaskBoard {
        TaskBoard { root_id: root.into(), path: String::new(), doc_revision: String::new(), unidentified_items: 0,
            diagnostics: vec![], tasks: vec![TaskView { task: Task { task_id: task.into(), title: "Task".into(),
                body: String::new(), description: String::new(), description_editable: true, description_diagnostic: None,
                steps: vec![], step_progress: None, steps_diagnostic: None, depends_on: vec![], follow_up_of: None,
                relations_diagnostic: None, checked: true, line: 1, task_revision: String::new(), diagnostic: None },
                project: None, lane: TaskLane::Ready, current_run_id: None,
                dependencies: TaskDependencies { state: TaskDependencyState::None, unmet: vec![], problems: vec![] },
                scheduled: false, attempt: None, waiting: None, worker_waiting: None }] }
    }
    #[test]
    fn task_lane_labels_are_neutral_and_preserve_other_states() {
        for (lane, label) in [
            (TaskLane::Blocked, "Blocked"),
            (TaskLane::Ready, "Ready"),
            (TaskLane::Working, "Assigned"),
            (TaskLane::Review, "Review"),
            (TaskLane::Accepted, "Accepted"),
        ] {
            assert_eq!(task_lane_label(lane), label);
        }
    }

    #[test]
    fn generated_task_text_describes_assignment_without_claiming_activity() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "r".into();
        worker.parent_run_id = Some("r".into());
        worker.task_id = Some("task".into());
        worker.assignment_generation = Some(1);
        worker.stage = RunStage::Working;
        let mut state = OrchestrationState { runs: vec![root("r"), worker], ..Default::default() };
        state.task_ownership.push(TaskOwnership { root_id: "r".into(), task_id: "task".into(), generation: 1, disposition: None, disposition_history: Vec::new(),
            schedule: None, reopened_after_attempt: None,
            owner: Some(TaskOwner { run_id: "worker".into(), conversation: None, claim_operation_id: "claim".into() }) });
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        state.messages.push(report("progress", "r", 1, Some("t"), ReportKind::Progress));
        let mut task_board = board("r", "task");
        task_board.tasks[0].task.checked = false;
        let projected = view(&state, vec![task_board]);
        let thread = &projected.threads[0];
        assert_eq!(thread.tasks[0].lane, TaskLane::Working);
        assert_eq!(thread.next.as_ref().unwrap().text, "Assigned · Task");
        assert!(thread.facts.iter().any(|fact| fact.text == "Assigned · Task"));
    }
    #[test]
    fn close_fences_are_session_scoped_and_reopen_on_widget_or_message() {
        use cockpit_protocol::widget::{WidgetKey, WidgetLiveBinding};
        use super::super::store::WidgetPublication;
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        state.messages.push(message("request", "r", 1, Some("t"), MessageKind::Instruction));
        let marker = |session: &str, messages, widgets| ThreadReadRecord {
            thread_id: String::new(),
            through_message_id: super::super::threads::close_marker(session, "t", messages, widgets),
            origin: ActorRef::Operator, at: NOW.into(),
        };
        state.thread_reads.push(marker("other", 1, 0));
        assert_eq!(view(&state, vec![]).threads[0].state, InboxState::Underway);
        state.thread_reads.push(marker("session", 1, 0));
        let closed = view(&state, vec![]);
        assert_eq!(closed.threads[0].state, InboxState::Handled);
        assert_eq!(closed.threads[0].closed_at.as_deref(), Some(NOW));
        let event = InboxWidgetEvent { key: WidgetKey { session_id: "session".into(), tab_id: "tab".into(), id: "w".into() },
            revision: 1, binding: WidgetLiveBinding { run_id: "r".into(), attempt: 1, session_id: "session".into(),
                attachment_id: "attachment".into(), attachment_epoch: 1 }, title: "Widget".into(),
            publication_id: "publication".into(), content_sha256: "hash".into() };
        state.widget_publications.push(WidgetPublication { thread_id: "t".into(), root_id: "r".into(),
            at: NOW.into(), message_position: 1, event });
        assert_eq!(view(&state, vec![]).threads[0].state, InboxState::Underway);
        state.thread_reads.push(marker("session", 1, 1));
        state.messages.push(report("update", "r", 2, Some("t"), ReportKind::Progress));
        assert_eq!(view(&state, vec![]).threads[0].state, InboxState::Underway);
        state.thread_reads.push(marker("session", 99, 99));
        assert_eq!(view(&state, vec![]).threads[0].state, InboxState::Underway);
    }

    #[test]
    fn follow_up_reopens_close_even_if_linked_task_is_now_accepted() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(),
            linked_by: ActorRef::Operator, at: NOW.into() });
        state.messages.push(message("request", "r", 1, Some("t"), MessageKind::Instruction));
        state.thread_reads.push(ThreadReadRecord { thread_id: String::new(),
            through_message_id: super::super::threads::close_marker("session", "t", 1, 0),
            origin: ActorRef::Operator, at: NOW.into() });
        state.messages.push(message("follow-up", "r", 2, Some("t"), MessageKind::Instruction));
        let projected = view(&state, vec![board("r", "task")]);
        assert_eq!(projected.threads[0].state, InboxState::Underway);
        assert!(projected.threads[0].closed_at.is_none());
        state.messages.push(report("question", "r", 3, Some("t"), ReportKind::NeedsInput));
        assert_eq!(view(&state, vec![board("r", "task")]).threads[0].state, InboxState::NeedsYou);
    }

    #[test]
    fn close_marker_cannot_be_a_read_receipt_even_when_ids_collide() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let key = super::super::threads::close_marker("session", "t", 1, 0);
        state.messages.push(report(&key, "r", 1, Some("t"), ReportKind::Result));
        state.thread_reads.push(ThreadReadRecord { thread_id: String::new(),
            through_message_id: key.clone(), origin: ActorRef::Operator, at: NOW.into() });
        let projected = view(&state, vec![]);
        assert_eq!(projected.threads[0].need, Some(InboxNeed::Answer));
        assert_eq!(projected.threads[0].unread_result_message_id.as_deref(), Some(key.as_str()));
    }

    #[test]
    fn explicitly_closed_threads_obey_handled_cap() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        for index in 0..51 {
            let thread = format!("thread-{index}");
            state.messages.push(message(&format!("message-{index}"), "r", index + 1, Some(&thread), MessageKind::Instruction));
        }
        for index in 0..51 {
            let thread = format!("thread-{index}");
            state.thread_reads.push(ThreadReadRecord { thread_id: String::new(),
                through_message_id: super::super::threads::close_marker("session", &thread, 51, 0),
                origin: ActorRef::Operator, at: NOW.into() });
        }
        let capped = view(&state, vec![]);
        assert_eq!(capped.threads.len(), 50);
        assert!(capped.threads.iter().all(|thread| thread.closed_at.is_some()));
        let all = project_uncapped(&InboxRequest { session_id: "session".into() }, &state, vec![],
            Err(InspectionError::new("offline", "unavailable")), NOW, "store").unwrap();
        assert_eq!(all.threads.len(), 51);
    }

    #[test]
    fn widget_publications_survive_storage_and_preserve_order_scope_and_cap() {
        use cockpit_protocol::widget::{WidgetKey, WidgetLiveBinding};
        use super::super::store::WidgetPublication;
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut request = message("request", "r", 1, Some("t"), MessageKind::Instruction);
        request.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        request.created_at = "2026-10-06T12:00:00Z".into();
        state.messages.push(request);
        let event = InboxWidgetEvent { key: WidgetKey { session_id: "session".into(), tab_id: "tab".into(), id: "w".into() },
            revision: 1, binding: WidgetLiveBinding { run_id: "r".into(), attempt: 1, session_id: "session".into(),
                attachment_id: "attachment".into(), attachment_epoch: 1 }, title: "Original".into(),
            publication_id: "incarnation-1".into(), content_sha256: "hash".into() };
        state.widget_publications.push(WidgetPublication { thread_id: "t".into(), root_id: "r".into(),
            at: NOW.into(), message_position: 1, event: event.clone() });
        let mut second = event.clone();
        second.publication_id = "incarnation-2".into();
        state.widget_publications.push(WidgetPublication { thread_id: "other".into(), root_id: "r".into(),
            at: NOW.into(), message_position: 1, event: second });
        let mut update = report("update", "r", 2, Some("t"), ReportKind::Progress);
        update.created_at = "2026-10-04T12:00:00Z".into();
        state.messages.push(update);
        let bytes = serde_json::to_vec(&state).unwrap();
        let mut restored: OrchestrationState = serde_json::from_slice(&bytes).unwrap();
        let projected = view(&restored, vec![]);
        let entries = &projected.threads[0].timeline;
        assert_eq!(entries.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![InboxEntryKind::Request, InboxEntryKind::Widget, InboxEntryKind::Update]);
        assert_eq!(entries[1].widget.as_ref().unwrap().publication_id, "incarnation-1");
        assert_eq!(entries[1].widget.as_ref().unwrap().title, "Original");
        for seq in 3..=123 { restored.messages.push(report(&format!("update-{seq}"), "r", seq, Some("t"), ReportKind::Progress)); }
        let projected = view(&restored, vec![]);
        assert_eq!(projected.threads[0].timeline.len(), 120);
        assert_eq!(projected.threads[0].earlier_widgets.as_ref().unwrap()[0].message_id, "incarnation-1");
    }

    #[test]
    fn timeline_orders_owner_reports_and_untagged_answers_and_caps() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut request = message("request", "r", 1, Some("t"), MessageKind::Instruction);
        request.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        let mut follow = message("follow", "r", 3, Some("t"), MessageKind::Instruction);
        follow.handoff = request.handoff.clone();
        let question = report("question", "r", 4, Some("t"), ReportKind::NeedsInput);
        let mut answer = message("answer", "r", 5, None, MessageKind::Answer);
        answer.in_reply_to = Some("question".into());
        let mut stale = report("stale", "r", 6, Some("t"), ReportKind::Result);
        stale.stale = true;
        state.messages.extend([request, report("update", "r", 2, Some("t"), ReportKind::Progress), follow, question, answer, stale]);
        let projected = view(&state, vec![]);
        let entries = &projected.threads[0].timeline;
        assert_eq!(entries.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![InboxEntryKind::Request, InboxEntryKind::Update, InboxEntryKind::FollowUp, InboxEntryKind::Question, InboxEntryKind::Answer]);
        assert_eq!(entries[4].in_reply_to.as_deref(), Some("question"));
        for seq in 7..=130 { state.messages.push(report(&format!("update-{seq}"), "r", seq, Some("t"), ReportKind::Progress)); }
        let projected = view(&state, vec![]);
        assert_eq!(projected.threads[0].timeline.len(), 120);
        assert_eq!(projected.threads[0].timeline_omitted, 9);
    }
    #[test]
    fn evidence_selects_latest_result_and_pairs_decisions() {
        let mut old = root("old");
        old.kind = RunKind::Worker; old.root_id = "r".into(); old.task_id = Some("task".into()); old.attempt = 1;
        old.result = report("old-result", "old", 1, None, ReportKind::Result).report;
        let mut selected = old.clone();
        selected.run_id = "selected".into(); selected.attempt = 2;
        selected.result = report("result", "selected", 2, None, ReportKind::Result).report;
        selected.result.as_mut().unwrap().summary = "Full result\nSee `/tmp/mock.html`, and /tmp/image.png.".into();
        let source = SourceOutput::NonCode { result_message_id: "result".into(), task_revision: "rev".into(), artifacts: vec!["/tmp/recorded.md".into()], focused_checks: vec![] };
        selected.source_output = Some(source.clone());
        selected.integration = Integration::Integrated { operation_id: "op".into(), accepted_result_message_id: "result".into(), accepted_task_revision: "rev".into(), source: Box::new(source), target_repository_key: "repo".into(), target_ref: "refs/heads/main".into(), target_commit: "abc".into(), method: IntegrationMethod::Ancestor, mappings: Box::new([]), checks: vec![], integrator_run_id: "r".into(), verified_at: NOW.into() };
        let mut newest = old.clone();
        newest.run_id = "newest".into(); newest.attempt = 3; newest.result = None;
        let mut state = OrchestrationState { runs: vec![root("r"), old, selected, newest], ..Default::default() };
        let question = report("q", "old", 3, None, ReportKind::NeedsInput);
        let mut answer = message("a", "old", 4, None, MessageKind::Answer);
        answer.in_reply_to = Some("q".into());
        state.messages.extend([question, answer]);
        let evidence = task_evidence(&state, "r", "task", None).unwrap();
        assert_eq!(evidence.run_id, "selected");
        assert_eq!(evidence.attempts, 3);
        assert_eq!(evidence.integration.unwrap().target, "main");
        assert!(evidence.artifacts.iter().any(|a| a.path == "/tmp/mock.html" && !a.recorded));
        assert!(evidence.artifacts.iter().any(|a| a.path == "/tmp/image.png" && !a.recorded));
        assert!(evidence.artifacts.iter().any(|a| a.path == "/tmp/recorded.md" && a.recorded));
        assert_eq!(evidence.decisions[0].answer.as_deref(), Some("a"));
        assert_eq!(evidence.decisions[0].by, Some(InboxDecisionBy::You));
    }
    #[test]
    fn task_title_precedes_request_and_request_truncates_at_word_boundary() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut request = message("request", "r", 1, Some("t"), MessageKind::Instruction);
        request.text = format!("{} finalwords", "word ".repeat(18));
        request.handoff = Some(HandoffRecord { task_id: Some("task".into()), task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        state.messages.push(request);
        let without_board = view(&state, vec![]);
        assert!(without_board.threads[0].title.text.ends_with("…"));
        assert!(without_board.threads[0].title.text.chars().count() <= 80);
        let with_board = view(&state, vec![board("r", "task")]);
        assert_eq!(with_board.threads[0].title.text, "Task");
        assert!(matches!(with_board.threads[0].title.source, InboxSource::Owner { .. }));
    }
    #[test]
    fn thread_card_title_preserves_request_across_follow_up() {
        let mut state = OrchestrationState { runs: vec![root("root")], ..Default::default() };
        let original = "Could you investigate why the parser fails?\nPlease preserve the full details.";
        let mut request = message("request", "root", 1, Some("thread"), MessageKind::Instruction);
        request.text = original.into();
        request.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        state.messages.push(request);
        assert_eq!(view(&state, vec![]).threads[0].title.text, original.lines().next().unwrap());
        let mut progress = report("progress", "root", 2, Some("thread"), ReportKind::Progress);
        progress.report.as_mut().unwrap().card = Some(ThreadCard { title: "Parser failure".into(), next: "Investigating".into(), recommendation: None, choices: vec![], project: None });
        state.messages.push(progress);
        let mut follow_up = message("follow-up", "root", 3, Some("thread"), MessageKind::Instruction);
        follow_up.text = "Please also check Unicode.".into();
        follow_up.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        state.messages.push(follow_up);
        let mut result = report("result", "root", 4, Some("thread"), ReportKind::Result);
        result.report.as_mut().unwrap().card = Some(ThreadCard { title: "Parser failure".into(), next: "Answer ready".into(), recommendation: None, choices: vec![], project: None });
        state.messages.push(result);
        let projected = view(&state, vec![]);
        let thread = &projected.threads[0];
        assert_eq!(thread.title.text, "Parser failure");
        assert!(matches!(thread.title.source, InboxSource::OwnerCard { .. }));
        assert_eq!(thread.request.as_ref().unwrap().text, original);
    }
    #[test]
    fn latest_valid_owner_card_titles_thread_without_replacing_request() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker;
        worker.root_id = "root".into();
        worker.parent_run_id = Some("root".into());
        let mut state = OrchestrationState { runs: vec![root("root"), worker], ..Default::default() };
        let mut request = message("request", "root", 1, Some("thread"), MessageKind::Instruction);
        request.text = "Original request\nFull details".into();
        request.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        let mut original = report("original", "root", 2, Some("thread"), ReportKind::Progress);
        original.report.as_mut().unwrap().card = Some(ThreadCard { title: "Original title".into(), next: "Working".into(), recommendation: None, choices: vec![], project: None });
        let mut renamed = report("renamed", "root", 3, Some("thread"), ReportKind::Progress);
        renamed.report.as_mut().unwrap().card = Some(ThreadCard { title: "Renamed title".into(), next: "Working".into(), recommendation: None, choices: vec![], project: None });
        let without_card = report("without-card", "root", 4, Some("thread"), ReportKind::Result);
        let mut stale = report("stale", "root", 5, Some("thread"), ReportKind::Progress);
        stale.stale = true;
        stale.report.as_mut().unwrap().card = Some(ThreadCard { title: "Stale title".into(), next: "Old".into(), recommendation: None, choices: vec![], project: None });
        let mut worker_report = report("worker-report", "worker", 6, Some("thread"), ReportKind::Progress);
        worker_report.report.as_mut().unwrap().card = Some(ThreadCard { title: "Worker title".into(), next: "Old".into(), recommendation: None, choices: vec![], project: None });
        state.messages.extend([worker_report, stale, without_card, renamed, original, request]);
        let projected = view(&state, vec![]);
        let thread = &projected.threads[0];
        assert_eq!(thread.title.text, "Renamed title");
        assert_eq!(thread.title.message_id.as_deref(), Some("renamed"));
        assert!(matches!(thread.title.source, InboxSource::OwnerCard { .. }));
        assert_eq!(thread.request.as_ref().unwrap().text, "Original request\nFull details");
    }
    #[test]
    fn scope_legacy_answers_and_workers_are_not_human_questions() {
        let mut worker = root("worker");
        worker.kind = RunKind::Worker; worker.root_id = "root".into(); worker.parent_run_id = Some("root".into());
        let mut state = OrchestrationState { runs: vec![root("root"), worker], ..Default::default() };
        state.messages.push(report("q1", "root", 1, Some("a"), ReportKind::NeedsInput));
        state.messages.push(report("q2", "root", 2, Some("b"), ReportKind::NeedsInput));
        state.messages.push(report("legacy", "root", 3, None, ReportKind::NeedsInput));
        state.messages.push(report("worker-q", "worker", 1, None, ReportKind::NeedsInput));
        let mut answer = message("answer", "root", 4, Some("legacy"), MessageKind::Answer);
        answer.in_reply_to = Some("legacy".into()); state.messages.push(answer);
        let result = view(&state, vec![]);
        assert_eq!(result.threads.len(), 3);
        assert_eq!(result.threads.iter().find(|t| t.thread_id == "legacy").unwrap().state, InboxState::Handled);
        assert_eq!(result.threads.iter().filter(|t| t.need == Some(InboxNeed::Decision)).count(), 2);
        assert!(result.threads.iter().all(|t| t.questions.iter().all(|q| q.run_id == "root")));
    }
    #[test]
    fn equal_question_ids_resolve_to_the_correct_sender_and_thread() {
        let mut state = OrchestrationState { runs: vec![root("a"), root("b")], ..Default::default() };
        let mut a = report("decision", "a", 1, Some("thread-a"), ReportKind::NeedsInput);
        a.report.as_mut().unwrap().summary = "Question A".into();
        a.report.as_mut().unwrap().card = Some(ThreadCard { title: "A".into(), next: String::new(), recommendation: None, choices: vec!["A choice".into()], project: None });
        let mut b = report("decision", "b", 2, Some("thread-b"), ReportKind::NeedsInput);
        b.report.as_mut().unwrap().summary = "Question B".into();
        b.report.as_mut().unwrap().card = Some(ThreadCard { title: "B".into(), next: String::new(), recommendation: None, choices: vec!["B choice".into()], project: None });
        state.messages.extend([a, b]);
        let mut answer = message("answer", "a", 3, Some("thread-a"), MessageKind::Answer);
        answer.in_reply_to = Some("decision".into());
        state.messages.push(answer);
        let result = view(&state, vec![]);
        let a = result.threads.iter().find(|t| t.thread_id == "thread-a").unwrap();
        let b = result.threads.iter().find(|t| t.thread_id == "thread-b").unwrap();
        assert!(matches!(a.questions[0].receipt, QuestionReceipt::AnswerDelivered { .. }));
        assert!(matches!(b.questions[0].receipt, QuestionReceipt::Unresolved));
        assert_eq!(b.need, Some(InboxNeed::Decision));
        assert_eq!(b.questions[0].run_id, "b");
        assert_eq!(b.questions[0].text.text, "Question B");
        assert_eq!(b.questions[0].choices, vec!["B choice"]);
        assert!(matches!(&b.questions[0].text.source, InboxSource::Owner { run_id, .. } if run_id == "b"));
        assert_eq!(b.questions[0].text.at.as_deref(), Some("2026-10-05T12:00:02Z"));
    }
    #[test]
    fn legacy_equal_ids_keep_both_questions_and_operator_receipt() {
        let mut state = OrchestrationState { runs: vec![root("a"), root("b")], ..Default::default() };
        let mut a = report("decision", "a", 1, None, ReportKind::NeedsInput);
        a.report.as_mut().unwrap().summary = "Question A".into();
        let mut b = report("decision", "b", 2, None, ReportKind::NeedsInput);
        b.report.as_mut().unwrap().summary = "Question B".into();
        b.report.as_mut().unwrap().card = Some(ThreadCard { title: "B".into(), next: String::new(), recommendation: None, choices: vec!["B choice".into()], project: None });
        state.messages.extend([a, b]);
        let mut answer = message("decision", "a", 3, Some("decision"), MessageKind::Answer);
        answer.in_reply_to = Some("decision".into());
        state.messages.push(answer);
        let result = view(&state, vec![]);
        let thread = result.threads.iter().find(|t| t.thread_id == "decision").unwrap();
        assert_eq!(thread.questions.len(), 2);
        let a = thread.questions.iter().find(|q| q.run_id == "a").unwrap();
        let b = thread.questions.iter().find(|q| q.run_id == "b").unwrap();
        assert!(matches!(a.receipt, QuestionReceipt::AnswerDelivered { .. }));
        assert!(matches!(b.receipt, QuestionReceipt::Unresolved));
        assert_eq!(b.text.text, "Question B");
        assert_eq!(b.choices, vec!["B choice"]);
        assert_eq!(thread.owner_root_ids, vec!["a", "b"]);
        assert_eq!(thread.receipts.len(), 1);
        assert_eq!(thread.facts[0].at.as_deref(), Some("2026-10-05T12:00:03Z"));
        assert_eq!(thread.need, Some(InboxNeed::Decision));
    }
    #[test]
    fn handoff_preserves_user_request_and_delivery_receipt() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut m = message("request", "r", 1, Some("t"), MessageKind::Instruction);
        m.text = "Original ask\nMore detail".into();
        m.handoff = Some(HandoffRecord { task_id: None, task_revision: None, attempt_run_id: None, binding: None, origin: OperatorOrigin::Browser });
        state.messages.push(m);
        let result = view(&state, vec![]);
        let t = &result.threads[0];
        assert_eq!(t.title.text, "Original ask");
        assert!(matches!(t.title.source, InboxSource::User));
        assert_eq!(t.request.as_ref().unwrap().text, "Original ask\nMore detail");
        assert_eq!(t.receipts[0].stage, DeliveryStage::Stored);
        assert_eq!(result.owners[0].undelivered, 1);
    }
    #[test]
    fn card_provenance_question_choices_and_stale_reports() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        let mut first = report("first", "r", 1, Some("t"), ReportKind::NeedsInput);
        first.report.as_mut().unwrap().card = Some(ThreadCard { title: "AI title".into(), next: "next".into(), recommendation: Some("recommend".into()), choices: vec!["old".into()], project: None });
        state.messages.push(first);
        state.messages.push(report("current", "r", 2, Some("t"), ReportKind::NeedsInput));
        let mut stale = report("stale", "r", 3, Some("t"), ReportKind::Result);
        stale.stale = true; state.messages.push(stale);
        let result = view(&state, vec![]); let t = &result.threads[0];
        assert_eq!(t.questions[0].question_message_id, "current");
        assert!(t.questions[0].choices.is_empty());
        assert!(matches!(t.title.source, InboxSource::OwnerCard { .. }));
        assert!(matches!(t.questions[0].text.source, InboxSource::Owner { .. }));
        assert_eq!(t.need, Some(InboxNeed::Decision));
    }
    #[test]
    fn result_read_and_multiowner_links_are_preserved() {
        let mut state = OrchestrationState { runs: vec![root("a"), root("b")], ..Default::default() };
        state.messages.push(report("a-result", "a", 1, Some("t"), ReportKind::Result));
        state.messages.push(report("b-result", "b", 2, Some("t"), ReportKind::Result));
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "a".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        let first = view(&state, vec![board("a", "task")]);
        assert_eq!(first.threads[0].owner_root_ids, vec!["a", "b"]);
        assert_eq!(first.threads[0].need, Some(InboxNeed::Answer));
        state.thread_reads.push(ThreadReadRecord { thread_id: "t".into(), through_message_id: "b-result".into(), origin: ActorRef::Operator, at: NOW.into() });
        let read = view(&state, vec![board("a", "task")]);
        assert_eq!(read.threads[0].state, InboxState::Handled);
        assert_eq!(read.threads[0].tasks[0].lane, TaskLane::Accepted);
        assert!(matches!(read.owners[0].availability, OwnerAvailability::Unknown));
        assert!(read.owners[0].location.is_none());
    }
    #[test]
    fn accepted_links_handle_only_when_every_link_resolves() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        state.messages.push(report("progress", "r", 1, Some("t"), ReportKind::Progress));
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "task".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        assert_eq!(view(&state, vec![board("r", "task")]).threads[0].state, InboxState::Handled);
        state.thread_links.push(ThreadLinkRecord { thread_id: "t".into(), root_id: "r".into(), task_id: "missing".into(), linked_by: ActorRef::Operator, at: NOW.into() });
        assert_eq!(view(&state, vec![board("r", "task")]).threads[0].state, InboxState::Underway);
    }
    #[test]
    fn handled_cap_does_not_drop_actionable_threads() {
        let mut state = OrchestrationState { runs: vec![root("r")], ..Default::default() };
        for n in 1..=55 {
            let id = format!("result-{n}");
            state.messages.push(report(&id, "r", n, Some(&id), ReportKind::Result));
            state.thread_reads.push(ThreadReadRecord { thread_id: id.clone(), through_message_id: id, origin: ActorRef::Operator, at: NOW.into() });
        }
        state.messages.push(report("question", "r", 56, Some("question"), ReportKind::NeedsInput));
        let result = view(&state, vec![]);
        assert_eq!(result.threads.len(), 51);
        assert_eq!(result.threads[0].need, Some(InboxNeed::Decision));
        assert_eq!(result.threads.iter().filter(|t| t.state == InboxState::Handled).count(), 50);
    }
}
