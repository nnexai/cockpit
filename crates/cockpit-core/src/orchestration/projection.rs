use std::collections::{HashMap, HashSet};

use cockpit_protocol::orchestration::*;
use cockpit_protocol::v1::ErrorResponse;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

use super::herdr::RuntimeView;
use super::store::OrchestrationState;
use crate::InspectionError;

fn current_attempt_candidate(run: &Run, old: Option<&Run>) -> bool {
    run.close_reason != Some(CloseReason::Superseded)
        && !(matches!(run.stage, RunStage::Proposed | RunStage::AwaitingPrepare)
            && old.is_some_and(|old| old.stage != RunStage::Closed))
}

/// The same canonical-attempt selection used by the board, including the
/// pre-Prepare replacement exception. Historical superseded attempts never own content.
pub(crate) fn current_task_run<'a>(
    state: &'a OrchestrationState,
    root_id: &str,
    task_id: &str,
) -> Option<&'a Run> {
    state.runs.iter()
        .filter(|run| run.root_id == root_id && run.task_id.as_deref() == Some(task_id)
            && current_attempt_candidate(run, run.supersedes_run_id.as_deref()
                .and_then(|id| state.runs.iter().find(|old| old.run_id == id))))
        .max_by(|left, right| attempt_order(left).cmp(&attempt_order(right)))
}

/// Projects the current main request and its latest explicit recipient-inbox answer.
/// Delivery receipts never change the run or imply that work has resumed.
pub(crate) fn question_status(run: &Run, inbox: &[&Message]) -> Option<QuestionStatus> {
    if run.stage == RunStage::Closed {
        return None;
    }
    let report = run
        .last_report
        .as_ref()
        .filter(|report| report.kind == ReportKind::NeedsInput)?;
    let answer = inbox
        .iter()
        .copied()
        .filter(|message| {
            message.to_run_id == run.run_id
                && message.kind == MessageKind::Answer
                && !message.stale
                && message.from_subagent_id.is_none()
                && message.in_reply_to.as_deref() == Some(report.message_id.as_str())
        })
        .max_by_key(|message| message.seq);
    let receipt = answer.map_or(QuestionReceipt::Unresolved, |message| {
        let answer = AnswerReceipt {
            sender: message.from.clone(),
            message_id: message.message_id.clone(),
            seq: message.seq,
            stage: message.stage,
            created_at: message.created_at.clone(),
            acked_at: message.acked_at.clone(),
        };
        match message.stage {
            DeliveryStage::Stored | DeliveryStage::Woken | DeliveryStage::Read => {
                QuestionReceipt::AnswerDelivered { answer }
            }
            DeliveryStage::Acked => QuestionReceipt::AnswerAcknowledged { answer },
        }
    });
    Some(QuestionStatus {
        run_id: run.run_id.clone(),
        question_message_id: report.message_id.clone(),
        asked_at: report.at.clone(),
        receipt,
    })
}

/// Joins durable intent and canonical Markdown with one fresh runtime observation.
/// No runtime evidence is written back, and no observation can complete a task.
pub(crate) fn snapshot(
    request: &OrchestrationSnapshotRequest,
    state: &OrchestrationState,
    tasks_token: String,
    boards: Vec<TaskBoard>,
    runtime: Result<RuntimeView, InspectionError>,
    now: &str,
) -> Result<OrchestrationSnapshot, InspectionError> {
    let clock = parse_time(now).ok_or_else(|| {
        InspectionError::new("orchestration_clock", "Snapshot time must be RFC 3339")
    })?;
    let roots: Vec<&Run> = state
        .runs
        .iter()
        .filter(|run| {
            run.session_id == request.session_id
                && run.run_id == run.root_id
                && run.parent_run_id.is_none()
        })
        .collect();
    let root_ids: HashSet<&str> = roots.iter().map(|run| run.run_id.as_str()).collect();
    let selected_root = match request.root_id.as_deref() {
        Some(root_id) if !root_ids.contains(root_id) => {
            return Err(InspectionError::new(
                "root_not_found",
                "Root is not in this session",
            ));
        }
        Some(root_id) => Some(root_id),
        None if roots.len() == 1 => Some(roots[0].run_id.as_str()),
        None => None,
    };
    let runs: Vec<&Run> = state
        .runs
        .iter()
        .filter(|run| {
            run.session_id == request.session_id && root_ids.contains(run.root_id.as_str())
        })
        .collect();
    let run_by_id: HashMap<&str, &Run> =
        runs.iter().map(|run| (run.run_id.as_str(), *run)).collect();
    let messages: Vec<&Message> = state
        .messages
        .iter()
        .filter(|message| run_by_id.contains_key(message.to_run_id.as_str()))
        .collect();
    let mut inboxes: HashMap<&str, Vec<&Message>> = HashMap::new();
    for message in &messages {
        inboxes
            .entry(message.to_run_id.as_str())
            .or_default()
            .push(message);
    }
    let intents: Vec<&TaskIntent> = state
        .task_intents
        .iter()
        .filter(|intent| root_ids.contains(intent.root_id.as_str()))
        .collect();
    let runtime = observe(&runs, runtime, now);
    let observations: HashMap<&str, &RunObservation> = match &runtime.0 {
        RuntimeObservation::Fresh { runs, .. } => runs
            .iter()
            .map(|observation| (observation.run_id.as_str(), observation))
            .collect(),
        RuntimeObservation::Unavailable { .. } => HashMap::new(),
    };
    let mut attention = Vec::new();
    let mut questions = Vec::new();
    for run in &runs {
        let inbox: &[&Message] = inboxes.get(run.run_id.as_str()).map_or(&[], Vec::as_slice);
        let question = question_status(run, inbox);
        derive_attention(
            run,
            inbox,
            question.as_ref(),
            observations.get(run.run_id.as_str()).copied(),
            clock,
            now,
            &mut attention,
        );
        if let Some(question) = question {
            questions.push(question);
        }
        if let Some(RunRetirement { state: RetirementState::Unknown { at, .. }, .. }) = &run.retirement {
            attention.push(Attention {
                kind: AttentionKind::RetirementUnconfirmed,
                run_id: Some(run.run_id.clone()),
                task_id: run.task_id.clone(),
                message_seq: None,
                since: at.clone(),
            });
        }
    }
    for intent in &intents {
        if intent.state == IntentState::Conflict {
            let since = run_by_id
                .get(intent.run_id.as_str())
                .map_or(now, |run| run.updated_at.as_str());
            attention.push(Attention {
                kind: AttentionKind::IntentConflict,
                run_id: Some(intent.run_id.clone()),
                task_id: Some(intent.task_id.clone()),
                message_seq: None,
                since: since.to_owned(),
            });
        }
    }
    let mut current_runs: HashMap<(&str, &str), &Run> = HashMap::new();
    for run in &runs {
        let Some(task_id) = run.task_id.as_deref() else {
            continue;
        };
        if !current_attempt_candidate(run, run.supersedes_run_id.as_deref()
            .and_then(|id| run_by_id.get(id).copied())) {
            continue;
        }
        let key = (run.root_id.as_str(), task_id);
        match current_runs.get_mut(&key) {
            Some(current) if attempt_order(run) > attempt_order(current) => *current = run,
            Some(_) => {}
            None => {
                current_runs.insert(key, run);
            }
        }
    }
    let intent_runs: HashSet<&str> = intents
        .iter()
        .map(|intent| intent.run_id.as_str())
        .collect();
    let board = match selected_root {
        None => None,
        Some(root_id) => {
            let mut board = boards
                .into_iter()
                .find(|board| board.root_id == root_id)
                .ok_or_else(|| {
                    InspectionError::new(
                        "task_not_found",
                        "Canonical task document was not supplied",
                    )
                })?;
            for view in &mut board.tasks {
                let current = current_runs
                    .get(&(root_id, view.task.task_id.as_str()))
                    .copied();
                view.current_run_id = current.map(|run| run.run_id.clone());
                view.lane = lane(&view.task, current, &intent_runs);
                if let Some(code) = view.task.diagnostic.as_deref() {
                    if !board
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == code)
                    {
                        board.diagnostics.push(ErrorResponse {
                            code: code.to_owned(),
                            message: "Canonical task document contains an invalid task identity"
                                .to_owned(),
                        });
                    }
                }
            }
            Some(board)
        }
    };
    let mut open_counts: HashMap<&str, u32> = HashMap::new();
    for run in &runs {
        if run.stage != RunStage::Closed {
            *open_counts.entry(run.root_id.as_str()).or_default() += 1;
        }
    }
    let mut attention_counts: HashMap<&str, u32> = HashMap::new();
    for item in &attention {
        if let Some(run) = item.run_id.as_deref().and_then(|id| run_by_id.get(id)) {
            *attention_counts.entry(run.root_id.as_str()).or_default() += 1;
        }
    }
    Ok(OrchestrationSnapshot {
        session_id: request.session_id.clone(),
        revision: state.revision,
        tasks_token,
        roots: roots
            .into_iter()
            .map(|root| RootSummary {
                root_id: root.run_id.clone(),
                label: root.label.clone(),
                kind: root.kind,
                open_runs: open_counts.get(root.run_id.as_str()).copied().unwrap_or(0),
                needs_you: attention_counts
                    .get(root.run_id.as_str())
                    .copied()
                    .unwrap_or(0),
            })
            .collect(),
        board,
        runs: runs.into_iter().cloned().collect(),
        messages: messages.into_iter().cloned().collect(),
        questions,
        subagents: state
            .subagents
            .iter()
            .filter(|agent| run_by_id.contains_key(agent.run_id.as_str()))
            .cloned()
            .collect(),
        intents: intents.into_iter().cloned().collect(),
        assignment_intents: super::assignments::snapshot_intents(state, &request.session_id),
        runtime: runtime.0,
        unmanaged_agents: runtime.1,
        attention,
    })
}

fn attempt_order(run: &Run) -> (bool, u32, &str, &str) {
    (
        run.stage != RunStage::Closed,
        run.attempt,
        &run.created_at,
        &run.run_id,
    )
}

fn lane(task: &Task, run: Option<&Run>, intent_runs: &HashSet<&str>) -> TaskLane {
    if task.checked {
        return TaskLane::Accepted;
    }
    let Some(run) = run else {
        return TaskLane::Queued;
    };
    match run.stage {
        RunStage::Preparing | RunStage::Initializing => TaskLane::Setup,
        RunStage::Ready => TaskLane::Ready,
        RunStage::Working => TaskLane::Working,
        RunStage::Reported => TaskLane::Review,
        RunStage::Closed
            if run.close_reason == Some(CloseReason::Accepted)
                && intent_runs.contains(run.run_id.as_str()) =>
        {
            TaskLane::Review
        }
        _ => TaskLane::Queued,
    }
}

fn observe(
    runs: &[&Run],
    runtime: Result<RuntimeView, InspectionError>,
    now: &str,
) -> (RuntimeObservation, Vec<UnmanagedAgent>) {
    let view = match runtime {
        Ok(view) => view,
        Err(error) => {
            return (
                RuntimeObservation::Unavailable {
                    error: ErrorResponse {
                        code: error.code,
                        message: error.message,
                    },
                },
                Vec::new(),
            );
        }
    };
    let panes: HashMap<&str, _> = view
        .panes
        .iter()
        .map(|pane| (pane.pane_id.as_str(), pane))
        .collect();
    let mut matched = HashSet::new();
    let mut kernel_boot_id = None;
    let observations = runs.iter().map(|run| {
        let mut observation = RunObservation {
            run_id: run.run_id.clone(), presence: Presence::Unobserved,
            workspace_id: None, workspace_label: None, tab_id: None, tab_label: None,
            pane_id: None, agent_status: None, state_changed_at: None,
            actual_omp: false,
        };
        let Some(location) = run.location.as_ref() else { return observation };
        if location.endpoint_identity != view.endpoint_identity {
            observation.presence = Presence::EndpointChanged;
            return observation;
        }
        if matches!((location.boot_id.as_deref(), view.boot_id.as_deref()), (Some(expected), Some(actual)) if expected != actual) {
            observation.presence = Presence::EndpointChanged;
            return observation;
        }
        let direct = panes.get(location.pane_id.as_str()).copied();
        let mut pane = direct.filter(|pane| {
            identity_matches(location.terminal_id.as_deref(), pane.terminal_id.as_deref())
                && optional_evidence_matches(location.native_session_id.as_deref(), pane.native_session_id.as_deref())
                && optional_evidence_matches(run.bound_omp_session.as_deref(), pane.native_session_id.as_deref())
        });
        if pane.is_none() {
            // A fresh, unique terminal match is runtime observation, not mutation authority.
            // Check native session too when Herdr exposes it; older Herdr omits this field.
            if let (Some(terminal), Some(session)) =
                (location.terminal_id.as_deref(), run.bound_omp_session.as_deref()) {
                let mut candidates = view.panes.iter().filter(|candidate| {
                    candidate.terminal_id.as_deref() == Some(terminal)
                        && optional_evidence_matches(Some(session), candidate.native_session_id.as_deref())
                        && optional_evidence_matches(location.native_session_id.as_deref(), candidate.native_session_id.as_deref())
                });
                let candidate = candidates.next();
                if candidates.next().is_none() { pane = candidate; }
            }
        }
        let Some(pane) = pane else {
            let evidence_missing = direct.is_some_and(|pane| {
                location.terminal_id.is_some() && pane.terminal_id.is_none()
            });
            if !evidence_missing { observation.presence = Presence::Missing; }
            return observation;
        };
        matched.insert(pane.pane_id.as_str());
        observation.presence = Presence::Present;
        observation.actual_omp = actual_omp(run, pane, &mut kernel_boot_id);
        observation.workspace_id = Some(pane.workspace_id.clone());
        observation.workspace_label = Some(pane.workspace_label.clone());
        observation.tab_id = Some(pane.tab_id.clone());
        observation.tab_label = Some(pane.tab_label.clone());
        observation.pane_id = Some(pane.pane_id.clone());
        observation.agent_status = pane.agent_status.clone();
        observation.state_changed_at = pane.state_changed_at.clone();
        observation
    }).collect();
    let unmanaged = view
        .panes
        .iter()
        .filter(|pane| !matched.contains(pane.pane_id.as_str()))
        .filter_map(|pane| {
            if pane.launch_pending {
                return None;
            }
            let kind = pane.agent_kind.as_deref().filter(|kind| !kind.is_empty())?;
            let name = pane.agent_name.as_deref().unwrap_or(kind);
            Some(UnmanagedAgent {
                workspace_id: pane.workspace_id.clone(),
                workspace_label: pane.workspace_label.clone(),
                tab_id: pane.tab_id.clone(),
                tab_label: pane.tab_label.clone(),
                pane_id: pane.pane_id.clone(),
                agent_name: name.to_owned(),
                agent_status: pane.agent_status.clone(),
                state_changed_at: pane.state_changed_at.clone(),
            })
        })
        .collect();
    (
        RuntimeObservation::Fresh {
            endpoint_identity: view.endpoint_identity,
            observed_at: now.to_owned(),
            runs: observations,
        },
        unmanaged,
    )
}

/// Shared fresh occupant proof for projection and unresolved-launch escalation.
/// Endpoint and available boot/native receipt fences are checked by the caller.
pub(super) fn actual_omp(
    run: &Run,
    pane: &super::herdr::RuntimePane,
    kernel_boot_id: &mut Option<Option<String>>,
) -> bool {
    let Some(location) = run.location.as_ref() else { return false };
    run.stage != RunStage::Closed
        && run.bound_omp_session.as_deref().is_some_and(|s| !s.is_empty())
        && pane.agent_kind.as_deref().is_some_and(super::NativeAgentKind::is_omp) && !pane.launch_pending
        && (run.kind == RunKind::Adopted || super::launch_receipt_coherent(run))
        && pane.pane_id == location.pane_id
        && pane.workspace_id == location.workspace_id && pane.tab_id == location.tab_id
        && location.terminal_id.is_some() && pane.terminal_id == location.terminal_id
        && (pane.native_session_id.as_deref() == run.bound_omp_session.as_deref()
            || (pane.native_session_id.is_none()
                && run.bound_omp_process.as_ref().is_some_and(|process| {
                    process.kernel_boot_id.is_some()
                        && super::retire::exact_running(process, kernel_boot_id
                            .get_or_insert_with(crate::process_identity::kernel_boot_id)
                            .as_deref()).unwrap_or(false)
                        && i32::try_from(process.pid).ok().is_some_and(|pid| {
                            crate::process_identity::incarnation_foreground(pid,
                                process.start_ticks).unwrap_or(false)
                        })
                })))
}

fn identity_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_none_or(|expected| actual == Some(expected))
}

fn optional_evidence_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    !matches!((expected, actual), (Some(expected), Some(actual)) if expected != actual)
}

fn derive_attention(
    run: &Run,
    inbox: &[&Message],
    question: Option<&QuestionStatus>,
    observation: Option<&RunObservation>,
    clock: OffsetDateTime,
    now: &str,
    attention: &mut Vec<Attention>,
) {
    if run.stage == RunStage::Closed {
        return;
    }
    let mut add = |kind, since: &str, seq| {
        attention.push(Attention {
            kind,
            run_id: Some(run.run_id.clone()),
            task_id: run.task_id.clone(),
            message_seq: seq,
            since: since.to_owned(),
        })
    };
    match run.stage {
        RunStage::AwaitingPrepare => add(AttentionKind::AwaitsPrepare, &run.updated_at, None),
        RunStage::Ready => add(AttentionKind::AwaitsExecute, &run.updated_at, None),
        RunStage::Reported => add(
            AttentionKind::ToAccept,
            run.result
                .as_ref()
                .map_or(&run.updated_at, |report| &report.at),
            None,
        ),
        _ => {}
    }
    if let Some(dispatch) = &run.dispatch {
        if matches!(
            dispatch.step,
            DispatchStep::SetupUnknown | DispatchStep::LaunchUnknown | DispatchStep::NeedsReview
        ) {
            add(AttentionKind::DispatchUnknown, &dispatch.updated_at, None);
        }
        if dispatch
            .error
            .as_ref()
            .is_some_and(|error| error.code == "plan_changed")
        {
            add(AttentionKind::PlanChanged, &dispatch.updated_at, None);
        }
    }
    let needs_input = question
        .filter(|question| matches!(&question.receipt, QuestionReceipt::Unresolved));
    if let Some(question) = needs_input {
        add(AttentionKind::NeedsInput, &question.asked_at, None);
    }
    let latest = |kind| {
        inbox
            .iter()
            .copied()
            .filter(|message| message.kind == kind && !message.stale)
            .max_by_key(|message| message.seq)
    };
    let prepare = latest(MessageKind::PrepareBrief);
    let work = latest(MessageKind::WorkBrief);
    for brief in [prepare, work].into_iter().flatten() {
        let since = if brief.kind == MessageKind::PrepareBrief {
            if run.location.is_none() {
                continue;
            }
            let launched = run
                .dispatch
                .as_ref()
                .filter(|dispatch| dispatch.step == DispatchStep::Launched)
                .map_or(run.updated_at.as_str(), |dispatch| {
                    dispatch.updated_at.as_str()
                });
            latest_time(&brief.created_at, launched)
        } else {
            brief.created_at.as_str()
        };
        if matches!(brief.stage, DeliveryStage::Stored | DeliveryStage::Woken)
            && older_than(clock, since, 120)
        {
            add(AttentionKind::BriefUnread, since, Some(brief.seq));
        }
    }
    let Some(observation) = observation else {
        return;
    };
    match observation.presence {
        Presence::Missing => add(AttentionKind::ExitedWithoutReport, now, None),
        Presence::EndpointChanged => {
            if !run.dispatch.as_ref().is_some_and(|dispatch| {
                matches!(
                    dispatch.step,
                    DispatchStep::SetupUnknown
                        | DispatchStep::LaunchUnknown
                        | DispatchStep::NeedsReview
                )
            }) {
                add(AttentionKind::DispatchUnknown, now, None);
            }
        }
        Presence::Present => {
            if observation.agent_status.as_deref() == Some("blocked") && needs_input.is_none() {
                add(
                    AttentionKind::RuntimeBlocked,
                    observation.state_changed_at.as_deref().unwrap_or(now),
                    None,
                );
            }
            if run.stage == RunStage::Working
                && run.result.is_none()
                && matches!(observation.agent_status.as_deref(), Some("idle" | "done"))
            {
                if let Some(brief) = work {
                    let mut since = brief.created_at.as_str();
                    if let Some(report) = &run.last_report {
                        since = latest_time(since, &report.at);
                    }
                    if let Some(changed) = &observation.state_changed_at {
                        since = latest_time(since, changed);
                    }
                    if older_than(clock, since, 300) {
                        add(AttentionKind::IdleWithoutReport, since, Some(brief.seq));
                    }
                }
            }
        }
        Presence::Unobserved => {}
    }
}

fn parse_time(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn older_than(now: OffsetDateTime, since: &str, seconds: i64) -> bool {
    parse_time(since).is_some_and(|since| now - since > Duration::seconds(seconds))
}

fn later_or_equal(left: &str, right: &str) -> bool {
    match (parse_time(left), parse_time(right)) {
        (Some(left), Some(right)) => left >= right,
        _ => false,
    }
}

fn latest_time<'a>(left: &'a str, right: &'a str) -> &'a str {
    if later_or_equal(right, left) {
        right
    } else {
        left
    }
}

#[cfg(test)]
mod tests {
    use super::super::herdr::RuntimePane;
    use super::*;

    const ROOT: &str = "11111111-1111-4111-8111-111111111111";
    const WORKER: &str = "22222222-2222-4222-8222-222222222222";
    const TASK: &str = "33333333-3333-4333-8333-333333333333";
    const START: &str = "2026-10-05T12:00:00Z";
    const NOW: &str = "2026-10-05T12:10:00Z";

    fn worker(stage: RunStage) -> Run {
        Run {
            session_id: "session".into(),
            prepare_brief: "read-only initialization".into(),
            run_id: WORKER.into(),
            kind: RunKind::Worker,
            label: "worker".into(),
            root_id: ROOT.into(),
            parent_run_id: Some(ROOT.into()),
            task_id: Some(TASK.into()),
            attempt: 1,
            task_revision_at_propose: Some("item-revision".into()),
            stage,
            close_reason: None,
            dispatch: Some(DispatchState {
                launch_tag: Some("launch".into()),
                endpoint_identity: Some("endpoint".into()),
                recovery: None,
                agent_started: true,
                step: DispatchStep::Launched,
                launch_attempt: 1,
                error: None,
                updated_at: START.into(),
            }),
            target: None,
            setup: None,
            prepare_plan: None,
            init_receipt: None,
            work_plan: None,
            grants: Vec::new(),
            last_report: None,
            result: None,
            annotations: Vec::new(),
            location: Some(RunLocation {
                endpoint_identity: "endpoint".into(),
                session_id: "session".into(),
                boot_id: None,
                terminal_id: None,
                native_session_id: None,
                workspace_id: "old-space".into(),
                tab_id: "old-tab".into(),
                pane_id: "pane".into(),
                launch_tag: "launch".into(),
            }),
            bound_omp_session: None,
            bound_omp_process: None,
            launch_shell_identity: None,
            retirement: None,
            supersedes_run_id: None,
            created_at: START.into(),
            updated_at: START.into(),
        }
    }

    fn state(run: Run) -> OrchestrationState {
        let mut root = worker(RunStage::Active);
        root.run_id = ROOT.into();
        root.kind = RunKind::Supervisor;
        root.parent_run_id = None;
        root.task_id = None;
        root.location = None;
        root.dispatch = None;
        OrchestrationState {
            runs: vec![root, run],
            revision: 7,
            ..Default::default()
        }
    }

    fn board() -> TaskBoard {
        TaskBoard {
            root_id: ROOT.into(),
            path: "/canonical/tasks/root.md".into(),
            doc_revision: "doc-revision".into(),
            unidentified_items: 0,
            diagnostics: Vec::new(),
            tasks: vec![TaskView {
                task: Task {
                    task_id: TASK.into(),
                    title: "Canonical title".into(),
                    body: "Canonical body".into(),
                    description: "Canonical body".into(),
                    description_editable: true,
                    description_diagnostic: None,
                    steps: Vec::new(),
                    step_progress: Some(TaskStepProgress { done: 0, total: 0 }),
                    steps_diagnostic: None,
                    depends_on: Vec::new(),
                    follow_up_of: None,
                    relations_diagnostic: None,
                    checked: false,
                    line: 3,
                    task_revision: "item-revision".into(),
                    diagnostic: None,
                },
                lane: TaskLane::Accepted,
                current_run_id: None,
                dependencies: TaskDependencies { state: TaskDependencyState::None, unmet: Vec::new(), problems: Vec::new() },
            }],
        }
    }

    fn runtime(status: &str) -> RuntimeView {
        RuntimeView {
            endpoint_identity: "endpoint".into(),
            boot_id: None,
            workspaces: Vec::new(),
            panes: vec![RuntimePane {
                workspace_id: "live-space".into(),
                workspace_label: "Live space".into(),
                tab_id: "live-tab".into(),
                tab_label: "Live tab".into(),
                pane_id: "pane".into(),
                terminal_id: None,
                native_session_id: None,
                agent_name: Some("worker".into()),
                agent_status: Some(status.into()),
                agent_kind: Some("omp".into()),
                launch_pending: false,
                interactive_ready: false,
                state_changed_at: Some(START.into()),
            }],
        }
    }

    fn brief(kind: MessageKind, stage: DeliveryStage) -> Message {
        Message {
            message_id: "44444444-4444-4444-8444-444444444444".into(),
            to_run_id: WORKER.into(),
            seq: 1,
            from: ActorRef::Operator,
            kind,
            text: "brief".into(),
            in_reply_to: None,
            report: None,
            stale: false,
            escalated_from: None,
            from_subagent_id: None,
            stage,
            woken_omp_session: None,
            created_at: START.into(),
            acked_at: None,
        }
    }

    fn asking() -> Run {
        let mut run = worker(RunStage::Working);
        run.last_report = Some(Report {
            message_id: "ask".into(),
            kind: ReportKind::NeedsInput,
            outcome: None,
            summary: "Need a decision".into(),
            plan: None,
            at: START.into(),
        });
        run
    }

    fn answer(stage: DeliveryStage) -> Message {
        let mut message = brief(MessageKind::Answer, stage);
        message.in_reply_to = Some("ask".into());
        if stage == DeliveryStage::Acked {
            message.acked_at = Some(NOW.into());
        }
        message
    }

    fn project(
        state: &OrchestrationState,
        runtime: Result<RuntimeView, InspectionError>,
    ) -> OrchestrationSnapshot {
        snapshot(
            &OrchestrationSnapshotRequest {
                session_id: "session".into(),
                root_id: None,
            },
            state,
            "token".into(),
            vec![board()],
            runtime,
            NOW,
        )
        .unwrap()
    }

    #[test]
    fn questions_are_recipient_scoped_for_all_open_runs_in_the_requested_session() {
        let mut durable = state(asking());
        durable.runs[0].last_report = durable.runs[1].last_report.clone();
        let mut foreign_root = asking();
        foreign_root.session_id = "foreign-session".into();
        foreign_root.run_id = "foreign-root".into();
        foreign_root.root_id = "foreign-root".into();
        foreign_root.parent_run_id = None;
        foreign_root.kind = RunKind::Supervisor;
        durable.runs.push(foreign_root);
        let mut root_answer = answer(DeliveryStage::Acked);
        root_answer.to_run_id = ROOT.into();
        durable.messages.push(root_answer);
        let projected = project(&durable, Ok(runtime("working")));
        assert_eq!(projected.questions.len(), 2);
        let root = projected.questions.iter().find(|question| question.run_id == ROOT).unwrap();
        let QuestionReceipt::AnswerAcknowledged { answer } = &root.receipt else {
            panic!("root answer is acknowledged")
        };
        assert_eq!(answer.acked_at.as_deref(), Some(NOW));
        let worker = projected.questions.iter().find(|question| question.run_id == WORKER).unwrap();
        assert!(matches!(&worker.receipt, QuestionReceipt::Unresolved));
        assert!(!projected.attention.iter().any(|item| {
            item.kind == AttentionKind::NeedsInput && item.run_id.as_deref() == Some(ROOT)
        }));
        assert!(projected.attention.iter().any(|item| {
            item.kind == AttentionKind::NeedsInput && item.run_id.as_deref() == Some(WORKER)
        }));
        assert!(projected.questions.iter().all(|question| question.run_id != "foreign-root"));
    }

    fn has(snapshot: &OrchestrationSnapshot, kind: AttentionKind) -> bool {
        snapshot
            .attention
            .iter()
            .any(|attention| attention.kind == kind)
    }

    #[test]
    fn unanswered_current_main_question_is_projected_without_changing_durable_facts() {
        let durable = state(asking());
        let before = serde_json::to_value(&durable).unwrap();
        let projected = project(&durable, Ok(runtime("working")));
        assert_eq!(projected.questions.len(), 1);
        let question = &projected.questions[0];
        assert_eq!(question.run_id, WORKER);
        assert_eq!(question.question_message_id, "ask");
        assert_eq!(question.asked_at, START);
        assert!(matches!(&question.receipt, QuestionReceipt::Unresolved));
        assert!(has(&projected, AttentionKind::NeedsInput));
        assert_eq!(projected.runs[1].stage, RunStage::Working);
        assert_eq!(serde_json::to_value(&durable).unwrap(), before);
    }

    #[test]
    fn linked_answer_projects_each_actual_delivery_stage_and_receipt() {
        for stage in [
            DeliveryStage::Stored,
            DeliveryStage::Woken,
            DeliveryStage::Read,
            DeliveryStage::Acked,
        ] {
            let mut durable = state(asking());
            let mut message = answer(stage);
            message.from = ActorRef::Run { run_id: ROOT.into() };
            message.seq = 8;
            // Identity, not wall-clock ordering, establishes the answer link.
            message.created_at = "2026-10-05T11:59:00Z".into();
            durable.messages.push(message.clone());
            let projected = project(&durable, Ok(runtime("working")));
            assert_eq!(projected.questions.len(), 1);
            let receipt = match &projected.questions[0].receipt {
                QuestionReceipt::AnswerDelivered { answer } => {
                    assert_ne!(stage, DeliveryStage::Acked);
                    answer
                }
                QuestionReceipt::AnswerAcknowledged { answer } => {
                    assert_eq!(stage, DeliveryStage::Acked);
                    answer
                }
                QuestionReceipt::Unresolved => panic!("linked answer is a receipt"),
            };
            assert!(matches!(&receipt.sender, ActorRef::Run { run_id } if run_id == ROOT));
            assert_eq!(receipt.message_id, message.message_id);
            assert_eq!(receipt.seq, 8);
            assert_eq!(receipt.stage, stage);
            assert_eq!(receipt.created_at, message.created_at);
            assert_eq!(receipt.acked_at, message.acked_at);
            assert!(!has(&projected, AttentionKind::NeedsInput));
            assert_eq!(projected.runs[1].last_report.as_ref().unwrap().message_id, "ask");
            assert_eq!(projected.runs[1].stage, RunStage::Working);
            assert!(projected.runs[1].result.is_none());
        }
    }

    #[test]
    fn newest_stored_correction_supersedes_older_acknowledged_answer_by_inbox_sequence() {
        let mut durable = state(asking());
        let mut acknowledged = answer(DeliveryStage::Acked);
        acknowledged.seq = 2;
        let mut correction = answer(DeliveryStage::Stored);
        correction.message_id = "correction".into();
        correction.seq = 3;
        correction.created_at = START.into();
        // Vector and timestamp ordering are not the recipient sequence ordering.
        durable.messages.extend([correction, acknowledged]);
        let projected = project(&durable, Ok(runtime("idle")));
        let QuestionReceipt::AnswerDelivered { answer } = &projected.questions[0].receipt else {
            panic!("correction has not been acknowledged")
        };
        assert_eq!(answer.message_id, "correction");
        assert_eq!(answer.seq, 3);
        assert_eq!(answer.stage, DeliveryStage::Stored);
        assert!(answer.acked_at.is_none());
        assert!(!has(&projected, AttentionKind::NeedsInput));
    }

    #[test]
    fn unrelated_stale_subagent_and_other_inbox_messages_do_not_resolve_question() {
        let baseline = answer(DeliveryStage::Acked);
        let mut old_question = baseline.clone();
        old_question.in_reply_to = Some("old-ask".into());
        let mut stale = baseline.clone();
        stale.stale = true;
        let mut subagent = baseline.clone();
        subagent.from_subagent_id = Some("internal-child".into());
        let mut instruction = baseline.clone();
        instruction.kind = MessageKind::Instruction;
        let mut other_inbox = baseline;
        other_inbox.to_run_id = ROOT.into();
        for mut message in [old_question, stale, subagent, instruction, other_inbox] {
            message.created_at = NOW.into();
            let mut durable = state(asking());
            durable.messages.push(message);
            let projected = project(&durable, Ok(runtime("working")));
            assert_eq!(projected.questions.len(), 1);
            assert!(matches!(&projected.questions[0].receipt, QuestionReceipt::Unresolved));
            assert!(has(&projected, AttentionKind::NeedsInput));
        }
    }

    #[test]
    fn new_question_does_not_inherit_old_answer_and_nonquestion_report_clears_receipt() {
        let mut durable = state(asking());
        durable.messages.push(answer(DeliveryStage::Acked));
        let report = durable.runs[1].last_report.as_mut().unwrap();
        report.message_id = "next-ask".into();
        // Same timestamp and wording still describe a distinct question.
        let next = project(&durable, Ok(runtime("working")));
        assert_eq!(next.questions.len(), 1);
        assert_eq!(next.questions[0].question_message_id, "next-ask");
        assert!(matches!(&next.questions[0].receipt, QuestionReceipt::Unresolved));
        assert!(has(&next, AttentionKind::NeedsInput));
        for kind in [ReportKind::Progress, ReportKind::Ready, ReportKind::Result] {
            durable.runs[1].last_report.as_mut().unwrap().kind = kind;
            let replaced = project(&durable, Ok(runtime("working")));
            assert!(replaced.questions.is_empty());
            assert!(!has(&replaced, AttentionKind::NeedsInput));
        }
    }

    #[test]
    fn closed_run_keeps_history_but_has_no_current_question() {
        let mut run = asking();
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Accepted);
        let mut durable = state(run);
        durable.messages.push(answer(DeliveryStage::Acked));
        let projected = project(&durable, Ok(runtime("working")));
        assert!(projected.questions.is_empty());
        assert!(!has(&projected, AttentionKind::NeedsInput));
        assert_eq!(projected.runs[1].last_report.as_ref().unwrap().message_id, "ask");
        assert_eq!(projected.messages.len(), 1);
    }

    #[test]
    fn report_messages_do_not_replace_the_current_main_question_receipt() {
        let mut durable = state(asking());
        durable.messages.push(answer(DeliveryStage::Acked));
        let mut subagent_report = brief(MessageKind::Report, DeliveryStage::Stored);
        subagent_report.message_id = "child-ask".into();
        subagent_report.from = ActorRef::Run { run_id: WORKER.into() };
        subagent_report.to_run_id = ROOT.into();
        subagent_report.from_subagent_id = Some("child".into());
        subagent_report.report = Some(Report {
            message_id: "child-ask".into(),
            kind: ReportKind::NeedsInput,
            outcome: None,
            summary: "A separate subagent question".into(),
            plan: None,
            at: NOW.into(),
        });
        durable.messages.push(subagent_report);
        let projected = project(&durable, Ok(runtime("working")));
        assert_eq!(projected.questions.len(), 1);
        assert_eq!(projected.questions[0].question_message_id, "ask");
        assert!(matches!(&projected.questions[0].receipt, QuestionReceipt::AnswerAcknowledged { .. }));
        durable.runs[1].last_report = None;
        assert!(project(&durable, Ok(runtime("working"))).questions.is_empty());
    }

    #[test]
    fn runtime_done_is_not_a_success_or_a_completion_report() {
        let mut durable = state(worker(RunStage::Working));
        durable
            .messages
            .push(brief(MessageKind::WorkBrief, DeliveryStage::Acked));
        let snapshot = project(&durable, Ok(runtime("done")));
        assert_eq!(
            snapshot.board.as_ref().unwrap().tasks[0].lane,
            TaskLane::Working
        );
        assert!(has(&snapshot, AttentionKind::IdleWithoutReport));
        assert!(!has(&snapshot, AttentionKind::ToAccept));
        assert!(snapshot.runs[1].result.is_none());
        assert_eq!(durable.runs[1].stage, RunStage::Working);
        assert_eq!(durable.revision, 7);
        let RuntimeObservation::Fresh { runs, .. } = &snapshot.runtime else {
            panic!("fresh observation")
        };
        let observed = runs.iter().find(|run| run.run_id == WORKER).unwrap();
        assert_eq!(observed.workspace_id.as_deref(), Some("live-space"));
        assert_eq!(observed.tab_id.as_deref(), Some("live-tab"));
    }

    #[test]
    fn initialization_receipt_is_separate_from_execute_grant() {
        let initializing = project(&state(worker(RunStage::Initializing)), Ok(runtime("idle")));
        assert_eq!(initializing.board.unwrap().tasks[0].lane, TaskLane::Setup);
        let mut run = worker(RunStage::Ready);
        run.init_receipt = Some(Report {
            message_id: "ready".into(),
            kind: ReportKind::Ready,
            outcome: None,
            summary: "Initialized".into(),
            plan: Some("Work plan".into()),
            at: START.into(),
        });
        run.work_plan = Some(PlanRecord {
            plan_revision: "work-plan".into(),
            text: "Work plan".into(),
            created_at: START.into(),
        });
        let ready = project(&state(run.clone()), Ok(runtime("idle")));
        assert_eq!(ready.board.as_ref().unwrap().tasks[0].lane, TaskLane::Ready);
        assert!(has(&ready, AttentionKind::AwaitsExecute));
        assert!(!has(&ready, AttentionKind::IdleWithoutReport));
        run.stage = RunStage::Working;
        run.grants.push(Grant {
            grant_id: "execute".into(),
            scope: GrantScope::Execute,
            plan_revision: "work-plan".into(),
            origin: GrantOrigin::Browser,
            supervisor_run_id: None,
            omp_session_id: None,
            granted_at: START.into(),
        });
        let working = project(&state(run), Ok(runtime("working")));
        assert_eq!(
            working.board.as_ref().unwrap().tasks[0].lane,
            TaskLane::Working
        );
        assert!(working.runs[1].init_receipt.is_some());
        assert!(!has(&working, AttentionKind::AwaitsExecute));
    }

    #[test]
    fn unread_brief_is_a_diagnostic_not_an_ack_or_completion() {
        let mut durable = state(worker(RunStage::Working));
        durable
            .messages
            .push(brief(MessageKind::WorkBrief, DeliveryStage::Woken));
        let unread = project(&durable, Ok(runtime("working")));
        assert!(has(&unread, AttentionKind::BriefUnread));
        assert_eq!(unread.board.unwrap().tasks[0].lane, TaskLane::Working);
        assert_eq!(durable.messages[0].stage, DeliveryStage::Woken);
        for delivered in [DeliveryStage::Read, DeliveryStage::Acked] {
            durable.messages[0].stage = delivered;
            let read = project(&durable, Ok(runtime("working")));
            assert!(!has(&read, AttentionKind::BriefUnread));
            assert_eq!(read.board.unwrap().tasks[0].lane, TaskLane::Working);
        }
    }

    #[test]
    fn prepare_brief_grace_starts_at_launch_not_proposal() {
        let mut run = worker(RunStage::Initializing);
        run.dispatch.as_mut().unwrap().updated_at = "2026-10-05T12:09:00Z".into();
        let mut durable = state(run);
        durable
            .messages
            .push(brief(MessageKind::PrepareBrief, DeliveryStage::Stored));
        assert!(!has(
            &project(&durable, Ok(runtime("idle"))),
            AttentionKind::BriefUnread
        ));
        durable.runs[1].dispatch.as_mut().unwrap().updated_at = START.into();
        assert!(has(
            &project(&durable, Ok(runtime("idle"))),
            AttentionKind::BriefUnread
        ));
    }

    #[test]
    fn unavailable_and_changed_endpoints_never_infer_completion() {
        let durable = state(worker(RunStage::Working));
        let unavailable = project(
            &durable,
            Err(InspectionError::new("offline", "unreachable")),
        );
        assert!(matches!(
            unavailable.runtime,
            RuntimeObservation::Unavailable { .. }
        ));
        assert!(!has(&unavailable, AttentionKind::ExitedWithoutReport));
        assert_eq!(unavailable.board.unwrap().tasks[0].lane, TaskLane::Working);
        let mut changed = runtime("done");
        changed.endpoint_identity = "replacement".into();
        let changed = project(&durable, Ok(changed));
        assert!(has(&changed, AttentionKind::DispatchUnknown));
        assert_eq!(changed.unmanaged_agents.len(), 1);
        let RuntimeObservation::Fresh { runs, .. } = &changed.runtime else {
            panic!("fresh observation")
        };
        assert_eq!(runs[1].presence, Presence::EndpointChanged);
        assert_eq!(changed.board.unwrap().tasks[0].lane, TaskLane::Working);
        let missing = project(
            &durable,
            Ok(RuntimeView {
                panes: Vec::new(),
                ..runtime("done")
            }),
        );
        assert!(has(&missing, AttentionKind::ExitedWithoutReport));
        assert_eq!(missing.board.unwrap().tasks[0].lane, TaskLane::Working);
    }

    #[test]
    fn proposed_replacement_does_not_displace_active_attempt() {
        let mut durable = state(worker(RunStage::Working));
        let mut replacement = worker(RunStage::AwaitingPrepare);
        replacement.run_id = "55555555-5555-4555-8555-555555555555".into();
        replacement.attempt = 2;
        replacement.supersedes_run_id = Some(WORKER.into());
        replacement.location = None;
        durable.runs.push(replacement);
        let old = project(&durable, Ok(runtime("working")));
        assert_eq!(
            old.board.as_ref().unwrap().tasks[0]
                .current_run_id
                .as_deref(),
            Some(WORKER)
        );
        assert_eq!(old.board.unwrap().tasks[0].lane, TaskLane::Working);
        durable.runs[1].stage = RunStage::Closed;
        durable.runs[1].close_reason = Some(CloseReason::Superseded);
        durable.runs[2].stage = RunStage::Preparing;
        assert_eq!(
            project(&durable, Ok(runtime("working")))
                .board
                .unwrap()
                .tasks[0]
                .lane,
            TaskLane::Setup
        );
    }

    #[test]
    fn intent_conflict_stays_in_review_and_markdown_check_wins() {
        let mut run = worker(RunStage::Closed);
        run.close_reason = Some(CloseReason::Accepted);
        let mut durable = state(run);
        durable.task_intents.push(TaskIntent {
            intent_id: "intent".into(),
            root_id: ROOT.into(),
            task_id: TASK.into(),
            run_id: WORKER.into(),
            expected_task_revision: "item-revision".into(),
            state: IntentState::Conflict,
            origin: None,
            supervisor_run_id: None,
            omp_session_id: None,
            result_message_id: None,
        });
        let conflicted = project(&durable, Ok(runtime("idle")));
        assert!(has(&conflicted, AttentionKind::IntentConflict));
        assert_eq!(conflicted.board.unwrap().tasks[0].lane, TaskLane::Review);
        let mut checked = board();
        checked.tasks[0].task.checked = true;
        let accepted = snapshot(
            &OrchestrationSnapshotRequest {
                session_id: "session".into(),
                root_id: None,
            },
            &durable,
            "token".into(),
            vec![checked],
            Ok(runtime("idle")),
            NOW,
        )
        .unwrap();
        assert_eq!(accepted.board.unwrap().tasks[0].lane, TaskLane::Accepted);
    }

    #[test]
    fn open_explicit_ask_supersedes_runtime_blocked_overlay() {
        let mut durable = state(asking());
        let blocked = project(&durable, Ok(runtime("blocked")));
        assert!(has(&blocked, AttentionKind::NeedsInput));
        assert!(!has(&blocked, AttentionKind::RuntimeBlocked));
        durable
            .messages
            .push(answer(DeliveryStage::Stored));
        let answered = project(&durable, Ok(runtime("blocked")));
        assert!(!has(&answered, AttentionKind::NeedsInput));
        assert!(has(&answered, AttentionKind::RuntimeBlocked));
    }

    #[test]
    fn roots_and_inbox_are_scoped_to_requested_session() {
        let mut durable = state(worker(RunStage::Reported));
        let mut foreign = worker(RunStage::Ready);
        foreign.session_id = "another-session".into();
        foreign.run_id = "foreign-root".into();
        foreign.root_id = foreign.run_id.clone();
        foreign.parent_run_id = None;
        foreign.task_id = None;
        durable.runs.push(foreign);
        let mut foreign_message = brief(MessageKind::SupervisorBrief, DeliveryStage::Stored);
        foreign_message.to_run_id = "foreign-root".into();
        durable.messages.push(foreign_message);
        let scoped = project(&durable, Ok(runtime("done")));
        assert_eq!(scoped.roots.len(), 1);
        assert_eq!(scoped.roots[0].open_runs, 2);
        assert_eq!(scoped.roots[0].needs_you, 1);
        assert_eq!(scoped.runs.len(), 2);
        assert!(scoped.messages.is_empty());
        assert_eq!(scoped.board.unwrap().tasks[0].lane, TaskLane::Review);
        let foreign_root = snapshot(
            &OrchestrationSnapshotRequest {
                session_id: "session".into(),
                root_id: Some("foreign-root".into()),
            },
            &durable,
            "token".into(),
            vec![board()],
            Ok(runtime("done")),
            NOW,
        );
        assert_eq!(foreign_root.unwrap_err().code, "root_not_found");
    }

    #[test]
    fn moved_pane_uses_unique_terminal_and_bound_session_without_mutating_receipt() {
        let mut run = worker(RunStage::Working);
        run.location.as_mut().unwrap().terminal_id = Some("terminal".into());
        run.location.as_mut().unwrap().native_session_id = Some("omp-session".into());
        run.bound_omp_session = Some("omp-session".into());
        let durable = state(run);
        let mut live = runtime("working");
        live.panes[0].pane_id = "moved-pane".into();
        live.panes[0].terminal_id = Some("terminal".into());
        live.panes[0].native_session_id = Some("omp-session".into());
        let moved = project(&durable, Ok(live.clone()));
        let RuntimeObservation::Fresh { runs, .. } = &moved.runtime else {
            panic!("fresh observation")
        };
        assert_eq!(runs[1].presence, Presence::Present);
        assert_eq!(runs[1].pane_id.as_deref(), Some("moved-pane"));
        assert_eq!(runs[1].workspace_id.as_deref(), Some("live-space"));
        assert!(moved.unmanaged_agents.is_empty());
        assert_eq!(durable.runs[1].location.as_ref().unwrap().pane_id, "pane");
        let mut duplicate = live.panes[0].clone();
        duplicate.pane_id = "ambiguous-pane".into();
        live.panes.push(duplicate);
        let ambiguous = project(&durable, Ok(live));
        let RuntimeObservation::Fresh { runs, .. } = &ambiguous.runtime else {
            panic!("fresh observation")
        };
        assert_eq!(runs[1].presence, Presence::Missing);
        assert!(runs[1].pane_id.is_none());
        assert_eq!(ambiguous.unmanaged_agents.len(), 2);
    }

    #[test]
    fn absent_terminal_evidence_cannot_claim_present() {
        let mut run = worker(RunStage::Working);
        run.location.as_mut().unwrap().terminal_id = Some("terminal".into());
        let unobserved = project(&state(run), Ok(runtime("working")));
        let RuntimeObservation::Fresh { runs, .. } = &unobserved.runtime else {
            panic!("fresh observation")
        };
        assert_eq!(runs[1].presence, Presence::Unobserved);
        assert!(runs[1].pane_id.is_none());
        assert_eq!(unobserved.unmanaged_agents.len(), 1);
    }

    #[test]
    fn bound_main_with_unavailable_optional_runtime_fields_is_managed_at_exact_location() {
        for receipt_has_optional_evidence in [false, true] {
            let mut run = worker(RunStage::Working);
            run.bound_omp_session = Some("omp-session".into());
            let location = run.location.as_mut().unwrap();
            location.terminal_id = Some("terminal".into());
            if receipt_has_optional_evidence {
                location.boot_id = Some("boot".into());
                location.native_session_id = Some("omp-session".into());
            }
            let mut live = runtime("working");
            live.panes[0].terminal_id = Some("terminal".into());
            let observed = project(&state(run), Ok(live));
            let RuntimeObservation::Fresh { runs, .. } = &observed.runtime else {
                panic!("fresh observation")
            };
            assert_eq!(runs[1].presence, Presence::Present);
            assert_eq!(runs[1].pane_id.as_deref(), Some("pane"));
            assert!(observed.unmanaged_agents.is_empty());
        }
    }

    #[test]
    fn moved_terminal_is_observed_when_herdr_omits_optional_native_session() {
        let mut run = worker(RunStage::Working);
        run.bound_omp_session = Some("omp-session".into());
        run.location.as_mut().unwrap().terminal_id = Some("terminal".into());
        let mut live = runtime("working");
        live.panes[0].terminal_id = Some("terminal".into());
        live.panes[0].pane_id = "moved-pane".into();
        let observed = project(&state(run), Ok(live));
        let RuntimeObservation::Fresh { runs, .. } = &observed.runtime else {
            panic!("fresh observation")
        };
        assert_eq!(runs[1].presence, Presence::Present);
        assert_eq!(runs[1].pane_id.as_deref(), Some("moved-pane"));
        assert!(observed.unmanaged_agents.is_empty());
    }

    #[test]
    fn actual_omp_requires_current_native_identity_not_a_display_name() {
        let mut run = worker(RunStage::Working);
        run.bound_omp_session = Some("native-main".into());
        let location = run.location.as_mut().unwrap();
        location.workspace_id = "live-space".into();
        location.tab_id = "live-tab".into();
        location.terminal_id = Some("terminal".into());
        for name in [None, Some("renamed"), Some("launch")] {
            for (kind, pending, native, expected) in [
                (Some("omp"), false, None, false),
                (Some("omp"), false, Some("native-main"), true),
                (Some("omp"), true, Some("native-main"), false),
                (None, false, Some("native-main"), false),
                (Some("claude"), false, Some("native-main"), false),
                (Some("omp"), false, Some("other-main"), false),
            ] {
                let mut live = runtime("working");
                live.panes[0].agent_name = name.map(str::to_owned);
                live.panes[0].agent_kind = kind.map(str::to_owned);
                live.panes[0].launch_pending = pending;
                live.panes[0].terminal_id = Some("terminal".into());
                live.panes[0].native_session_id = native.map(str::to_owned);
                let observed = project(&state(run.clone()), Ok(live));
                let RuntimeObservation::Fresh { runs, .. } = observed.runtime else {
                    panic!("fresh observation")
                };
                assert_eq!(runs[1].actual_omp, expected);
            }
        }
        for mismatch in 0..6 {
            let mut run = run.clone();
            let mut live = runtime("working");
            live.panes[0].terminal_id = Some("terminal".into());
            live.panes[0].native_session_id = Some("native-main".into());
            match mismatch {
                0 => run.bound_omp_session = None,
                1 => live.panes[0].terminal_id = Some("replacement-terminal".into()),
                2 => live.panes[0].pane_id = "replacement-pane".into(),
                3 => live.panes[0].workspace_id = "replacement-space".into(),
                4 => live.panes[0].tab_id = "replacement-tab".into(),
                _ => run.stage = RunStage::Closed,
            }
            let observed = project(&state(run), Ok(live));
            let RuntimeObservation::Fresh { runs, .. } = observed.runtime else {
                panic!("fresh observation")
            };
            assert!(!runs[1].actual_omp, "mismatch {mismatch}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn actual_omp_without_native_session_rejects_stale_bound_process_identity() {
        use cockpit_protocol::orchestration::NativeProcessIdentity;
        let mut run = worker(RunStage::Active);
        run.bound_omp_session = Some("native-main".into());
        let location = run.location.as_mut().unwrap();
        location.workspace_id = "live-space".into();
        location.tab_id = "live-tab".into();
        location.terminal_id = Some("terminal".into());
        let pid = std::process::id();
        let current = NativeProcessIdentity {
            pid, start_ticks: crate::process_identity::start_identity(pid as i32).unwrap(),
            kernel_boot_id: crate::process_identity::kernel_boot_id(),
        };
        for mismatch in 0..4 {
            let mut identity = current.clone();
            match mismatch {
                0 => identity.start_ticks += 1,
                1 => identity.kernel_boot_id = Some("replacement-boot".into()),
                2 => identity.kernel_boot_id = None,
                _ => identity.pid = u32::MAX,
            }
            run.bound_omp_process = Some(identity);
            let mut live = runtime("working");
            live.panes[0].agent_name = None;
            live.panes[0].terminal_id = Some("terminal".into());
            let observed = project(&state(run.clone()), Ok(live));
            let RuntimeObservation::Fresh { runs, .. } = observed.runtime else {
                panic!("fresh observation")
            };
            assert!(!runs[1].actual_omp, "mismatch {mismatch}");
        }
    }

    #[test]
    fn unmanaged_agents_exclude_pending_registrations_and_include_detected_unnamed_omp() {
        for (kind, pending, name, count) in [
            (None, false, Some("registered-name"), 0),
            (Some("omp"), true, Some("pending-name"), 0),
            (Some("omp"), false, None, 1),
        ] {
            let mut live = runtime("working");
            live.panes[0].pane_id = "unmanaged-pane".into();
            live.panes[0].agent_kind = kind.map(str::to_owned);
            live.panes[0].launch_pending = pending;
            live.panes[0].agent_name = name.map(str::to_owned);
            let observed = project(&state(worker(RunStage::Working)), Ok(live));
            assert_eq!(observed.unmanaged_agents.len(), count);
            if count != 0 {
                assert_eq!(observed.unmanaged_agents[0].agent_name, "omp");
            }
        }
    }
}
