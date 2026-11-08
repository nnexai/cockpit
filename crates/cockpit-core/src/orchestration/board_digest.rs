use cockpit_protocol::orchestration::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::herdr::RuntimeView;
use crate::InspectionError;

#[derive(Debug, Clone, Serialize)]
pub struct BoardDigest {
    pub root_id: String, pub key: String, pub generated_at: String,
    pub runtime_observed: bool, pub open_work: u32, pub text: String,
    pub tasks: Vec<BoardDigestTask>, pub standing_directives: Vec<Directive>,
    pub retained: Vec<BoardDigestResource>, pub omitted_tasks: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct BoardDigestTask {
    pub task_id: String, pub title: String, pub project: Option<String>, pub thread_ids: Vec<String>,
    pub run_id: Option<String>, pub attempt: Option<u32>, pub worker: WorkerState,
    pub last_activity_at: Option<String>, pub placement: Option<String>,
    pub directive_ids: Vec<String>, pub counts_as_open: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct BoardDigestResource {
    pub resource_id: String, pub run_id: String, pub kind: String, pub state: String, pub reason: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState { Unscheduled, Queued, Starting, Working, IdleNoReport, Blocked, QuestionPending, AwaitingReview, WaitingOnUser, Gone, Unknown }

pub const RECONCILE_CHECKLIST: &str = "Reconcile (board quiet, nothing is working): for each open row decide one action — review an awaiting Result (accept/send back), answer or escalate a pending question, replace a gone or idle-no-report worker (task restart), schedule or cancel unscheduled/queued work, decide kept resources. Do not ping working workers. If nothing can progress without the user, ask once on the relevant thread and stop.";

fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}
fn placement(target: &DispatchTarget) -> String {
    match target {
        DispatchTarget::SpaceWorktree { .. } => "worktree",
        DispatchTarget::ExistingSpace { .. } => "space",
        DispatchTarget::OwnedAttemptDirectory { .. } => "attempt directory",
        DispatchTarget::Setup { .. } => "workspace setup",
    }.into()
}
fn shorten(s: &str, n: usize) -> String {
    let mut chars = s.chars();
    let mut out: String = chars.by_ref().take(n).collect();
    if chars.next().is_some() && n > 0 { out.pop(); out.push('…'); }
    out.replace(['\n', '\r'], " ")
}
fn priority(s: WorkerState) -> u8 {
    match s { WorkerState::AwaitingReview => 0, WorkerState::QuestionPending => 1, WorkerState::Gone => 2,
        WorkerState::IdleNoReport => 3, WorkerState::Blocked => 4, WorkerState::Unscheduled => 5,
        WorkerState::Queued => 6, WorkerState::Starting => 7, WorkerState::Unknown => 8,
        WorkerState::Working => 9, WorkerState::WaitingOnUser => 10 }
}
fn age(now: &str, at: Option<&str>) -> String {
    let delta = at.and_then(|at| Some(OffsetDateTime::parse(now, &Rfc3339).ok()? - OffsetDateTime::parse(at, &Rfc3339).ok()?));
    delta.map(|d| format!(" {}m", d.whole_minutes().max(0))).unwrap_or_default()
}
fn add_line(text: &mut String, line: &str) -> bool {
    if text.len() + line.len() + 1 > 4096 { return false; }
    if !text.is_empty() { text.push('\n'); }
    text.push_str(line); true
}

pub fn digest(snapshot: &OrchestrationSnapshot, root_id: &str, runtime: Option<&RuntimeView>, now: &str) -> Result<BoardDigest, InspectionError> {
    let board = snapshot.board.as_ref().filter(|b| b.root_id == root_id)
        .ok_or_else(|| InspectionError::new("root_not_found", "Root board is not available in snapshot"))?;
    let mut rows = Vec::new();
    let mut key_rows = Vec::new();
    let mut standing: Vec<_> = snapshot.directives.iter().filter(|d| d.root_id == root_id && d.removed_at.is_none() && matches!(d.scope, DirectiveScope::Standing { .. })).cloned().collect();
    standing.sort_by(|a,b| a.directive_id.cmp(&b.directive_id));
    for view in &board.tasks {
        if view.lane == TaskLane::Accepted { continue; }
        let id = &view.task.task_id;
        let run = view.current_run_id.as_deref().and_then(|id| snapshot.runs.iter().find(|r| r.run_id == id));
        let schedule = snapshot.task_ownership.iter().find(|o| o.root_id == root_id && o.task_id == *id).and_then(|o| o.schedule.as_ref());
        let mut threads: Vec<String> = snapshot.thread_links.iter().filter(|link| link.root_id == root_id && link.task_id == *id).map(|link| link.thread_id.clone()).chain(snapshot.messages.iter().filter(|m| m.handoff.as_ref().is_some_and(|h| h.task_id.as_deref() == Some(id))).filter_map(|m| m.thread_id.clone())).collect();
        threads.sort(); threads.dedup();
        let waiting_user = snapshot.questions.iter().any(|q| q.run_id == root_id && matches!(q.receipt, QuestionReceipt::Unresolved) && q.thread_id.as_ref().is_some_and(|t| threads.contains(t)));
        let mut questions: Vec<_> = snapshot.questions.iter().filter(|q| matches!(q.receipt, QuestionReceipt::Unresolved) && (run.is_some_and(|r| q.run_id == r.run_id) || (q.run_id == root_id && q.thread_id.as_ref().is_some_and(|t| threads.contains(t))))).map(|q| q.question_message_id.clone()).collect();
        questions.sort();
        let observation = run.and_then(|r| match &snapshot.runtime { RuntimeObservation::Fresh { runs, .. } if runtime.is_some() => runs.iter().find(|o| o.run_id == r.run_id), _ => None });
        let worker = if waiting_user { WorkerState::WaitingOnUser }
            else if !questions.is_empty() { WorkerState::QuestionPending }
            else if let Some(r) = run {
                if r.stage == RunStage::Reported { WorkerState::AwaitingReview }
                else if matches!(r.stage, RunStage::Queued | RunStage::Starting) || r.dispatch.as_ref().is_some_and(|d| !d.agent_started) { WorkerState::Starting }
                else if r.stage == RunStage::Closed { if schedule.is_some() { WorkerState::Queued } else { WorkerState::Unscheduled } }
                else { match observation {
                    Some(o) if matches!(o.presence, Presence::Missing | Presence::EndpointChanged) => WorkerState::Gone,
                    Some(o) if o.presence == Presence::Present => match o.agent_status.as_deref() { Some("working") => WorkerState::Working, Some("idle" | "done") => WorkerState::IdleNoReport, Some("blocked") => WorkerState::Blocked, _ => WorkerState::Unknown },
                    None if runtime.is_some() && matches!(&snapshot.runtime, RuntimeObservation::Fresh { .. }) => WorkerState::Gone,
                    _ => WorkerState::Unknown,
                }}
            } else if schedule.is_some() { WorkerState::Queued } else { WorkerState::Unscheduled };
        let mut directive_ids: Vec<_> = snapshot.directives.iter().filter(|d| d.root_id == root_id && d.removed_at.is_none() && matches!(&d.scope, DirectiveScope::Task { task_id } if task_id == id)).map(|d| d.directive_id.clone()).collect();
        directive_ids.sort();
        let last_activity_at = run.map(|r| r.updated_at.as_str()).into_iter().chain(observation.and_then(|o| o.state_changed_at.as_deref())).chain(snapshot.messages.iter().filter(|m| run.is_some_and(|r| m.to_run_id == r.run_id || matches!(&m.from, ActorRef::Run { run_id } if run_id == &r.run_id))).map(|m| m.created_at.as_str())).max().map(str::to_owned);
        let target = run.and_then(|r| r.target.as_ref()).or_else(|| schedule.and_then(|s| s.target.as_ref()));
        let mut place = target.map(placement);
        if let Some(hold) = schedule.and_then(|s| s.hold.as_ref()) { place = Some(format!("{} hold:{}", place.unwrap_or_default(), hold.code).trim().to_owned()); }
        key_rows.push(serde_json::json!([id, worker, run.map(|r| &r.run_id), run.map(|r| r.attempt), run.map(|r| r.stage), run.and_then(|r| r.result.as_ref()).map(|r| &r.message_id), questions, schedule.and_then(|s| s.hold.as_ref()).map(|h| &h.code)]));
        rows.push(BoardDigestTask { task_id: id.clone(), title: view.task.title.clone(), project: view.project.as_ref().map(|p| p.label.clone()), thread_ids: threads, run_id: run.map(|r| r.run_id.clone()), attempt: run.map(|r| r.attempt), worker, last_activity_at, placement: place, directive_ids, counts_as_open: !waiting_user });
    }
    rows.sort_by(|a,b| priority(a.worker).cmp(&priority(b.worker)).then_with(|| a.task_id.cmp(&b.task_id)));
    let mut retained = Vec::new();
    for summary in &snapshot.cleanup {
        if !snapshot.runs.iter().any(|r| r.root_id == root_id && r.run_id == summary.run_id) { continue; }
        for item in &summary.items {
            if !matches!(item.cleanup.state, CleanupState::Waiting | CleanupState::WaitingIntegration | CleanupState::Kept | CleanupState::Unknown) { continue; }
            retained.push(BoardDigestResource { resource_id: item.resource_id.clone(), run_id: summary.run_id.clone(), kind: name(&item.kind), state: name(&item.cleanup.state), reason: item.cleanup.reason.as_ref().map(|r| r.code.clone()) });
        }
    }
    retained.sort_by(|a,b| a.resource_id.cmp(&b.resource_id));
    key_rows.sort_by_key(|row| row[0].as_str().unwrap_or_default().to_owned());
    let mut ids: Vec<_> = snapshot.directives.iter().filter(|d| d.root_id == root_id && d.removed_at.is_none()).map(|d| d.directive_id.as_str()).collect(); ids.sort();
    let resources: Vec<_> = retained.iter().map(|r| (&r.resource_id, &r.state, &r.reason)).collect();
    let canonical = serde_json::to_vec(&(key_rows, ids, resources)).expect("serializable digest key");
    let key = format!("{:x}", Sha256::digest(canonical))[..16].to_owned();
    let open_work = rows.iter().filter(|r| r.counts_as_open).count().saturating_add(retained.len()).min(u32::MAX as usize) as u32;
    let mut text = format!("Board {key} · {open_work} open · {} retained · runtime {}", retained.len(), if runtime.is_some() { "fresh" } else { "unknown" });
    for d in &standing {
        let scope = match &d.scope { DirectiveScope::Standing { project_key } => project_key.as_deref().unwrap_or("all"), _ => unreachable!() };
        if !add_line(&mut text, &format!("Standing: [{}] {} ({})", shorten(&d.directive_id, 24), shorten(&d.text, 160), shorten(scope, 40))) { break; }
    }
    let mut displayed = 0;
    for row in rows.iter().take(25) {
        let line = format!("- \"{}\"{}{} · {}{}{}{}", shorten(&row.title, 80), row.project.as_ref().map(|p| format!(" ({})", shorten(p, 40))).unwrap_or_default(), row.placement.as_ref().map(|p| format!(" {}", shorten(p, 80))).unwrap_or_default(), name(&row.worker).replace('_', "-"), age(now, row.last_activity_at.as_deref()), row.run_id.as_ref().map(|r| format!(" · run {}#{}", shorten(r, 12), row.attempt.unwrap_or(0))).unwrap_or_default(), if row.thread_ids.is_empty() { String::new() } else { format!(" · thread {}", shorten(&row.thread_ids.join(","), 80)) });
        if !add_line(&mut text, &line) { break; } displayed += 1;
        for directive in row.directive_ids.iter().filter_map(|id| snapshot.directives.iter().find(|d| &d.directive_id == id)) {
            if !add_line(&mut text, &format!("  user directive [{}]: {}", shorten(&directive.directive_id, 8), shorten(&directive.text, 160))) { break; }
        }
    }
    for r in &retained { if !add_line(&mut text, &format!("Retained: {} {} {}{}", r.kind, shorten(&r.run_id, 12), r.state, r.reason.as_ref().map(|s| format!(": {}", shorten(s, 80))).unwrap_or_default())) { break; } }
    Ok(BoardDigest { root_id: root_id.into(), key, generated_at: now.into(), runtime_observed: runtime.is_some(), open_work, text, omitted_tasks: (rows.len() - displayed).min(u32::MAX as usize) as u32, tasks: rows, standing_directives: standing, retained })
}

#[cfg(test)]
mod tests;
