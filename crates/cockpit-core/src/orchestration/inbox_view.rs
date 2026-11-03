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
    InboxText { text: value, source, message_id: message.map(|m| m.message_id.clone()), at: message.map(|m| m.created_at.clone()) }
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

fn question_message<'a>(messages: &'a [Message], q: &QuestionStatus) -> Option<&'a Message> {
    messages.iter().find(|m| m.message_id == q.question_message_id
        && matches!(&m.from, ActorRef::Run { run_id } if run_id == &q.run_id)
        && m.thread_id == q.thread_id)
}

pub(crate) fn project(request: &InboxRequest, state: &OrchestrationState, boards: Vec<TaskBoard>,
    runtime: Result<RuntimeView, InspectionError>, now: &str, store_id: &str) -> Result<InboxView, InspectionError> {
    let roots: Vec<&Run> = state.runs.iter().filter(|r| r.session_id == request.session_id
        && r.run_id == r.root_id && r.parent_run_id.is_none()).collect();
    let root_map: HashMap<&str, &Run> = roots.iter().map(|r| (r.run_id.as_str(), *r)).collect();
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
        if (root_map.contains_key(m.to_run_id.as_str()) && matches!(m.from, ActorRef::Operator)) || owner_report {
            if let Some(id) = m.thread_id.as_deref() { groups.entry(id).or_default().push(m); }
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
            task_projects.entry((m.to_run_id.as_str(), task)).or_insert_with(|| projection::task_project(state, &m.to_run_id, task, runtime_ref));
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
        }
        let request_message = messages.iter().copied().find(|m| m.handoff.is_some());
        let request_echo = request_message.map(|m| InboxRequestEcho { message_id: m.message_id.clone(), text: m.text.clone(), created_at: m.created_at.clone(), to_run_id: m.to_run_id.clone(), handoff: m.handoff.clone().unwrap() });
        let reports: Vec<(&Message, &Run)> = messages.iter().filter_map(|m| report_owner(m, &root_map).map(|r| (*m, r))).collect();
        let latest_report = reports.last().copied();
        let latest_card = reports.iter().rev().find(|(m, _)| m.report.as_ref().and_then(|r| r.card.as_ref()).is_some()).copied();
        let mut projected_questions = Vec::new();
        if let Some(statuses) = questions.get(id) { for q in statuses {
            if let (Some(m), Some(root)) = (question_message(&state.messages, q), root_map.get(q.run_id.as_str())) {
                if let Some(report) = &m.report { projected_questions.push(InboxQuestion { run_id: q.run_id.clone(), question_message_id: q.question_message_id.clone(), asked_at: q.asked_at.clone(), text: text(report.summary.clone(), owner_source(root, false), Some(m)), choices: report.card.as_ref().map(|c| c.choices.clone()).unwrap_or_default(), receipt: q.receipt.clone() }); }
            }
        }}
        projected_questions.sort_by(|a,b| (&a.asked_at, &a.run_id, &a.question_message_id).cmp(&(&b.asked_at, &b.run_id, &b.question_message_id)));
        let receipts: Vec<InboxReceipt> = messages.iter().filter(|m| matches!(m.from, ActorRef::Operator)).map(|m| InboxReceipt { message_id: m.message_id.clone(), kind: m.kind, to_run_id: m.to_run_id.clone(), attempt_run_id: m.handoff.as_ref().and_then(|h| h.attempt_run_id.clone()), stage: m.stage, created_at: m.created_at.clone(), acked_at: m.acked_at.clone(), stale: m.stale }).collect();
        let mut links = BTreeSet::new();
        for link in &state.thread_links { if link.thread_id == id && root_map.contains_key(link.root_id.as_str()) { links.insert((link.root_id.as_str(), link.task_id.as_str())); } }
        for m in &messages { if let Some(task) = m.handoff.as_ref().and_then(|h| h.task_id.as_deref()) { links.insert((m.to_run_id.as_str(), task)); } }
        let link_count = links.len();
        let mut tasks = Vec::new();
        for (root_id, task_id) in links {
            let Some(view) = boards.iter().find(|b| b.root_id == root_id).and_then(|b| b.tasks.iter().find(|t| t.task.task_id == task_id)) else { continue };
            let current = projection::current_task_run(state, root_id, task_id);
            let lane = projection::lane(&view.task, &view.dependencies, current, &intents);
            let run = current.map(|r| {
                let o = observations.get(r.run_id.as_str()).copied();
                InboxRun { run_id: r.run_id.clone(), attempt: r.attempt, stage: r.stage, harness: r.launch_config.harness_kind.clone(), presence: o.map_or(Presence::Unobserved, |o| o.presence), agent_status: o.and_then(|o| o.agent_status.clone()), location: location(o), cleanup: cleanups.iter().find(|c| c.run_id == r.run_id).map(|c| c.items.clone()).unwrap_or_default() }
            });
            tasks.push(InboxTaskLink { root_id: root_id.into(), task_id: task_id.into(), title: view.task.title.clone(), project: task_projects.get(&(root_id, task_id)).cloned().flatten(), lane, run });
        }
        let unread = latest_report.filter(|(m, _)| m.report.as_ref().is_some_and(|r| r.kind == ReportKind::Result)).filter(|(m, _)| !state.thread_reads.iter().filter(|read| read.thread_id == id).any(|read| {
            state.messages.iter().find(|mark| mark.message_id == read.through_message_id).is_some_and(|mark| mark.to_run_id == m.to_run_id && mark.seq >= m.seq)
        })).map(|(m, _)| m.message_id.clone());
        let unresolved = projected_questions.iter().any(|q| matches!(q.receipt, QuestionReceipt::Unresolved));
        let answered_legacy = questions.get(id).is_some_and(|qs| qs.iter().any(|q| q.thread_id.is_none() && !matches!(q.receipt, QuestionReceipt::Unresolved))) && latest_report.is_some_and(|(m, _)| m.message_id == id);
        let (state_value, need) = if unresolved { (InboxState::NeedsYou, Some(InboxNeed::Decision)) }
            else if unread.is_some() { (InboxState::NeedsYou, Some(InboxNeed::Answer)) }
            else if latest_report.is_some_and(|(m, _)| m.report.as_ref().is_some_and(|r| r.kind == ReportKind::Result)) || (link_count > 0 && tasks.len() == link_count && tasks.iter().all(|t| t.lane == TaskLane::Accepted)) || answered_legacy { (InboxState::Handled, None) }
            else { (InboxState::Underway, None) };
        let title = if let Some((m, root)) = latest_card { text(m.report.as_ref().unwrap().card.as_ref().unwrap().title.clone(), owner_source(root, true), Some(m)) }
            else if let Some(m) = request_message { text(first_line(&m.text), InboxSource::User, Some(m)) }
            else if let Some(q) = projected_questions.first() { text(first_line(&q.text.text), q.text.source.clone(), messages.iter().copied().find(|m| m.message_id == q.question_message_id && matches!(&m.from, ActorRef::Run { run_id } if run_id == &q.run_id))) }
            else if let Some((m, root)) = latest_report { text(first_line(&m.report.as_ref().unwrap().summary), owner_source(root, false), Some(m)) }
            else { text(id.to_owned(), InboxSource::Recorded, None) };
        let next = if let Some((m, root)) = latest_card { Some(text(m.report.as_ref().unwrap().card.as_ref().unwrap().next.clone(), owner_source(root, true), Some(m))) }
            else if let Some(receipt) = receipts.last() { let label = root_map.get(receipt.to_run_id.as_str()).map_or(receipt.to_run_id.as_str(), |r| r.label.as_str()); Some(text(if receipt.stage == DeliveryStage::Stored { format!("Waiting for {label} to pick up") } else { format!("With {label}") }, InboxSource::Recorded, None)) }
            else if tasks.len() == 1 { Some(text(format!("{:?} · {}", tasks[0].lane, tasks[0].title), InboxSource::Recorded, None)) } else { None };
        let recommendation = reports.iter().rev().find_map(|(m, root)| m.report.as_ref()?.card.as_ref()?.recommendation.as_ref().map(|v| text(v.clone(), owner_source(root, true), Some(m))));
        let reply = reports.iter().rev().find(|(m, _)| m.report.as_ref().is_some_and(|r| matches!(r.kind, ReportKind::Progress | ReportKind::Result))).map(|(m, root)| text(m.report.as_ref().unwrap().summary.clone(), owner_source(root, false), Some(m)));
        let project = latest_card.and_then(|(m, _)| m.report.as_ref()?.card.as_ref()?.project.clone());
        let mut facts = Vec::new();
        for r in &receipts { facts.push(text(format!("Delivery {:?}", r.stage), InboxSource::Recorded, messages.iter().copied().find(|m| m.message_id == r.message_id && m.to_run_id == r.to_run_id && matches!(m.from, ActorRef::Operator)))); }
        for t in &tasks {
            facts.push(text(format!("{:?} · {}", t.lane, t.title), InboxSource::Recorded, None));
            if let Some(run) = &t.run {
                for item in &run.cleanup { if matches!(item.cleanup.state, CleanupState::Removed | CleanupState::Kept) { facts.push(text(format!("Cleanup {:?}: {}", item.cleanup.state, item.cleanup.reason.as_ref().map_or("", |r| r.message.as_str())), InboxSource::Recorded, None)); } }
                if let Some(worker) = state.runs.iter().find(|r| r.run_id == run.run_id) {
                    let integration = match &worker.integration { Integration::Pending => "Pending", Integration::NotApplicable { .. } => "Not applicable", Integration::Integrated { .. } => "Integrated" };
                    facts.push(text(format!("Integration: {integration}"), InboxSource::Recorded, None));
                    if let Some(result) = &worker.result { facts.push(text(first_line(&result.summary), InboxSource::Worker { run_id: worker.run_id.clone() }, None)); }
                    if let Some(output) = &worker.source_output {
                        let checks = match output { SourceOutput::Code { focused_checks, .. } | SourceOutput::NonCode { focused_checks, .. } => focused_checks };
                        for check in checks { facts.push(text(format!("{}: {:?}", check.command_or_scenario, check.outcome), InboxSource::Worker { run_id: worker.run_id.clone() }, None)); }
                    }
                }
            }
        }
        facts.truncate(8);
        let updated_at = messages.iter().map(|m| m.created_at.as_str()).max().unwrap_or(now).to_owned();
        threads.push(InboxThread { thread_id: id.into(), state: state_value, need, owner_root_ids: owner_ids.into_iter().map(str::to_owned).collect(), title, next, project, request: request_echo, questions: projected_questions, recommendation, reply, unread_result_message_id: unread, tasks, receipts, facts, updated_at });
    }
    threads.sort_by(|a,b| {
        let rank = |t: &InboxThread| match t.need { Some(InboxNeed::Decision) => 0, Some(InboxNeed::Answer) => 1, None if t.state == InboxState::Underway => 2, None => 3 };
        rank(a).cmp(&rank(b)).then_with(|| if rank(a) == 0 { a.questions.iter().filter(|q| matches!(q.receipt, QuestionReceipt::Unresolved)).map(|q| &q.asked_at).min().cmp(&b.questions.iter().filter(|q| matches!(q.receipt, QuestionReceipt::Unresolved)).map(|q| &q.asked_at).min()) } else if rank(a) == 1 {
            let result_time = |t: &InboxThread| t.unread_result_message_id.as_deref().and_then(|id| state.messages.iter().find(|m| m.message_id == id)).map(|m| m.created_at.as_str());
            result_time(a).cmp(&result_time(b))
        } else { b.updated_at.cmp(&a.updated_at) }).then_with(|| a.thread_id.cmp(&b.thread_id))
    });
    let mut handled = 0;
    threads.retain(|t| { if t.state != InboxState::Handled { true } else { handled += 1; handled <= 50 } });
    let observation = match observation { RuntimeObservation::Fresh { observed_at, .. } => InboxObservation::Fresh { observed_at }, RuntimeObservation::Unavailable { error } => InboxObservation::Unavailable { error } };
    Ok(InboxView { session_id: request.session_id.clone(), state_store_id: store_id.into(), revision: state.revision, tasks_token: String::new(), observation, owners, threads, unmanaged_agents })
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
    fn message(id: &str, root: &str, seq: u64, thread: Option<&str>, kind: MessageKind) -> Message {
        Message { message_id: id.into(), to_run_id: root.into(), seq, from: ActorRef::Operator,
            kind, text: id.into(), in_reply_to: None, report: None, thread_id: thread.map(str::to_owned),
            handoff: None, stale: false, escalated_from: None, stage: DeliveryStage::Stored,
            created_at: format!("2026-10-05T12:{:02}:{:02}Z", seq / 60, seq % 60), acked_at: None }
    }
    fn report(id: &str, root: &str, seq: u64, thread: Option<&str>, kind: ReportKind) -> Message {
        let mut m = message(id, root, seq, thread, MessageKind::Report);
        m.from = ActorRef::Run { run_id: root.into() };
        m.report = Some(Report { message_id: id.into(), kind, outcome: None, summary: format!("summary {id}"),
            plan: None, source_output: None, card: None, at: m.created_at.clone() });
        m
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
                scheduled: false, attempt: None }] }
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
