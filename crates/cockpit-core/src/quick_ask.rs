pub mod herdr;

#[cfg(test)]
mod tests;

use std::{collections::HashMap, path::Path, sync::{Arc, Mutex}, time::{Duration, SystemTime, UNIX_EPOCH}};
use cockpit_protocol::quick_ask::{QuickAskAction, QuickAskActionRequest, QuickAskAgentAvailability, QuickAskAvailability, QuickAskDelivery, QuickAskFailure, QuickAskHarness, QuickAskLocation, QuickAskPhase, QuickAskStartRequest, QuickAskStatus};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, Notify};
use crate::InspectionError;
use herdr::{AgentStart, LaunchError, PromptError, QuickAskBinding, QuickAskHerdr};

const HARNESS: [(QuickAskHarness, &str, &str, &str); 6] = [
    (QuickAskHarness::Omp, "omp", "omp", "OMP"),
    (QuickAskHarness::Pi, "pi", "pi", "Pi"),
    (QuickAskHarness::Claude, "claude", "claude", "Claude Code"),
    (QuickAskHarness::Copilot, "copilot", "copilot", "Copilot CLI"),
    (QuickAskHarness::Codex, "codex", "codex", "Codex CLI"),
    (QuickAskHarness::Opencode, "opencode", "opencode", "OpenCode"),
];

struct Operation {
    status: QuickAskStatus,
    fingerprint: [u8; 32],
    cancelled: bool,
    manual_only: bool,
}
struct Entry { state: Mutex<Operation>, notify: Notify }

pub struct QuickAskService {
    adapter: Arc<dyn QuickAskHerdr>,
    start_lock: AsyncMutex<()>,
    entries: AsyncMutex<HashMap<String, Arc<Entry>>>,
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn terminal(phase: QuickAskPhase) -> bool { matches!(phase, QuickAskPhase::InitialQuestionSupplied | QuickAskPhase::Submitted | QuickAskPhase::Cancelled | QuickAskPhase::Failed | QuickAskPhase::OutcomeUnknown) }
fn error(code: &str, message: &str) -> InspectionError { InspectionError::new(code, message) }
fn set_phase(op: &mut Operation, phase: QuickAskPhase) {
    op.status.phase = phase;
    op.status.delivery = match phase { QuickAskPhase::InitialQuestionSupplied => QuickAskDelivery::InitialQuestionSupplied, QuickAskPhase::Submitted => QuickAskDelivery::Submitted, QuickAskPhase::OutcomeUnknown => QuickAskDelivery::Unknown, _ => QuickAskDelivery::NotSent };
    op.status.cancellable = !terminal(phase) && phase != QuickAskPhase::Sending;
    op.status.updated_at_ms = now();
}
fn finish(entry: &Entry, phase: QuickAskPhase, failure: Option<InspectionError>) {
    let mut op = entry.state.lock().unwrap();
    if op.cancelled && op.status.phase != QuickAskPhase::Sending {
        set_phase(&mut op, QuickAskPhase::Cancelled);
        return;
    }
    // Never expose upstream messages, which may contain sensitive input.
    op.status.error = failure.map(|e| QuickAskFailure { code: e.code, message: "The quick ask could not be completed. The tab may still be available.".into() });
    set_phase(&mut op, phase);
}
fn cancelled(entry: &Entry) -> bool {
    let mut op = entry.state.lock().unwrap();
    if op.cancelled { set_phase(&mut op, QuickAskPhase::Cancelled); true } else { false }
}
fn phase(entry: &Entry, value: QuickAskPhase) -> bool {
    let mut op = entry.state.lock().unwrap();
    if op.cancelled { set_phase(&mut op, QuickAskPhase::Cancelled); false } else { set_phase(&mut op, value); true }
}
fn validate(request: &QuickAskStartRequest) -> Result<(), InspectionError> {
    let id = uuid::Uuid::parse_str(&request.request_id).map_err(|_| error("quick_ask_invalid_request", "Request ID must be a lowercase hyphenated UUID"))?;
    if id.hyphenated().to_string() != request.request_id { return Err(error("quick_ask_invalid_request", "Request ID must be a lowercase hyphenated UUID")); }
    crate::validate_resource_id(&request.space_id, "space")?;
    if !Path::new(&request.cwd).is_absolute() || !Path::new(&request.cwd).is_dir() { return Err(error("quick_ask_invalid_cwd", "Working directory must be an existing absolute directory")); }
    let text = &request.prompt;
    if text.is_empty() || text.len() > 16384 || text.trim().is_empty() || text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(error("quick_ask_invalid_prompt", "Question must be 1–16384 UTF-8 bytes without control characters other than line feed"));
    }
    Ok(())
}

fn uses_initial_prompt(request: &QuickAskStartRequest) -> bool {
    matches!(request.harness, QuickAskHarness::Copilot | QuickAskHarness::Opencode)
        || (!matches!(request.prompt.as_bytes().first(), Some(b'@' | b'-'))
            && request.prompt.bytes().any(|byte| matches!(byte, b' ' | b'\n')))
}
fn executable_found(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| {
        let Ok(metadata) = std::fs::metadata(dir.join(name)) else { return false };
        if !metadata.is_file() { return false; }
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; metadata.permissions().mode() & 0o111 != 0 }
        #[cfg(not(unix))] { true }
    }))
}

impl QuickAskService {
    pub fn new(adapter: Arc<dyn QuickAskHerdr>) -> Self { Self { adapter, start_lock: AsyncMutex::new(()), entries: AsyncMutex::new(HashMap::new()) } }
    pub fn availability(&self) -> QuickAskAvailability {
        QuickAskAvailability { agents: HARNESS.iter().map(|(h, _, exe, _)| QuickAskAgentAvailability { harness: *h, executable: (*exe).into(), found: executable_found(exe) }).collect() }
    }
    pub async fn start(&self, session_id: &str, request: QuickAskStartRequest) -> Result<QuickAskStatus, InspectionError> {
        crate::validate_session_id(session_id)?;
        validate(&request)?;
        let mut digest = Sha256::new();
        for value in [session_id, &request.request_id, &request.server_instance, &request.space_id, &request.cwd, &request.prompt] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        digest.update([request.harness as u8]);
        let fingerprint: [u8; 32] = digest.finalize().into();
        let _start = self.start_lock.lock().await;
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get(&request.request_id) {
            let op = entry.state.lock().unwrap();
            return if op.fingerprint == fingerprint { Ok(op.status.clone()) } else { Err(error("quick_ask_request_conflict", "Request ID already belongs to another request")) };
        }
        let cutoff = now().saturating_sub(30 * 60 * 1000);
        entries.retain(|_, entry| { let op = entry.state.lock().unwrap(); !terminal(op.status.phase) || op.status.updated_at_ms >= cutoff });
        let active = entries.values().filter(|entry| !terminal(entry.state.lock().unwrap().status.phase)).count();
        if active >= 4 { return Err(error("quick_ask_busy", "Too many quick asks are in progress")); }
        let mut completed: Vec<_> = entries.iter().filter_map(|(id, entry)| { let op = entry.state.lock().unwrap(); terminal(op.status.phase).then(|| (id.clone(), op.status.updated_at_ms)) }).collect();
        completed.sort_by_key(|(_, timestamp)| *timestamp);
        for (id, _) in completed.iter().take(completed.len().saturating_sub(63)) { entries.remove(id); }
        drop(entries);
        let binding = self.adapter.bind(session_id, &request.server_instance, &request.space_id).await?;
        let status = QuickAskStatus { request_id: request.request_id.clone(), session_id: session_id.into(), server_instance: request.server_instance.clone(), harness: request.harness, phase: QuickAskPhase::CreatingTab, delivery: QuickAskDelivery::NotSent, cancellable: true, location: None, error: None, updated_at_ms: now() };
        let entry = Arc::new(Entry { state: Mutex::new(Operation { status: status.clone(), fingerprint, cancelled: false, manual_only: !uses_initial_prompt(&request) && request.prompt.contains('\n') }), notify: Notify::new() });
        let mut entries = self.entries.lock().await;
        entries.insert(request.request_id.clone(), entry.clone());
        let adapter = self.adapter.clone();
        tokio::spawn(async move { run(adapter, entry, binding, request).await; });
        Ok(status)
    }
    pub async fn status(&self, session_id: &str, request_id: &str) -> Result<QuickAskStatus, InspectionError> {
        let entries = self.entries.lock().await;
        let entry = entries.get(request_id).ok_or_else(|| error("quick_ask_request_unknown", "Quick ask request not found"))?;
        let op = entry.state.lock().unwrap();
        if op.status.session_id != session_id { return Err(error("quick_ask_request_unknown", "Quick ask request not found")); }
        Ok(op.status.clone())
    }
    pub async fn action(&self, session_id: &str, request: QuickAskActionRequest) -> Result<QuickAskStatus, InspectionError> {
        let entries = self.entries.lock().await;
        let entry = entries.get(&request.request_id).ok_or_else(|| error("quick_ask_request_unknown", "Quick ask request not found"))?;
        let mut op = entry.state.lock().unwrap();
        if op.status.session_id != session_id { return Err(error("quick_ask_request_unknown", "Quick ask request not found")); }
        match request.action {
            QuickAskAction::Cancel if op.status.cancellable => { op.cancelled = true; op.status.cancellable = false; op.status.updated_at_ms = now(); }
            QuickAskAction::Cancel => return Err(error("quick_ask_cancel_too_late", "Quick ask can no longer be cancelled")),
            QuickAskAction::Send if op.status.phase == QuickAskPhase::ReadyToSend && op.manual_only => return Err(error("quick_ask_manual_multiline", "Copy and enter this multiline question manually in the verified agent prompt")),
            QuickAskAction::Send if op.status.phase == QuickAskPhase::ReadyToSend && !op.cancelled => set_phase(&mut op, QuickAskPhase::Sending),
            QuickAskAction::Send => return Err(error("quick_ask_invalid_action", "Quick ask is not ready to send")),
        }
        entry.notify.notify_one();
        Ok(op.status.clone())
    }
}

async fn run(adapter: Arc<dyn QuickAskHerdr>, entry: Arc<Entry>, binding: QuickAskBinding, request: QuickAskStartRequest) {
    let (_, kind, _, label) = HARNESS.iter().find(|(h, _, _, _)| *h == request.harness).unwrap();
    let tag = request.request_id.replace('-', "");
    let tab_label = format!("Ask · {label} · {}", &tag[..4]);
    let name = format!("ask-{kind}-{}", &tag[..8]);
    let initial_prompt = uses_initial_prompt(&request);
    {
        let mut op = entry.state.lock().unwrap();
        if op.cancelled { set_phase(&mut op, QuickAskPhase::Cancelled); return; }
        // Supplying a startup question is itself the delivery boundary.
        if initial_prompt { set_phase(&mut op, QuickAskPhase::Sending); }
    }
    let pane = match adapter.create_tab(&binding, &request.cwd, &tab_label, kind, &name, initial_prompt.then_some(request.prompt.as_str())).await {
        Ok(p) => p,
        Err(LaunchError::NotSent(e)) => { finish(&entry, QuickAskPhase::Failed, Some(e)); return; }
        Err(LaunchError::Unknown(e)) => {
            finish(&entry, if initial_prompt { QuickAskPhase::OutcomeUnknown } else { QuickAskPhase::Failed }, Some(e));
            return;
        }
    };
    { let mut op = entry.state.lock().unwrap(); op.status.location = Some(QuickAskLocation { space_id: request.space_id.clone(), tab_id: pane.tab_id.clone(), pane_id: pane.pane_id.clone(), terminal_id: pane.terminal_id.clone(), tab_label, agent_name: String::new() }); }
    if initial_prompt {
        finish(&entry, QuickAskPhase::InitialQuestionSupplied, None);
        return;
    }
    if !phase(&entry, QuickAskPhase::StartingAgent) { return; }
    let (agent, wait, wait_seconds) = match adapter.start_agent(&binding, &pane, kind, &name).await {
        Ok(AgentStart::Ready(agent)) => (agent, false, 0),
        Ok(AgentStart::Blocked(agent)) => (agent, true, 600),
        Err(e) => { if !cancelled(&entry) { finish(&entry, QuickAskPhase::Failed, Some(e)); } return; }
    };
    { let mut op = entry.state.lock().unwrap(); if let Some(location) = &mut op.status.location { location.agent_name = agent.name.clone(); } }
    if cancelled(&entry) { return; }
    if wait {
        if !phase(&entry, QuickAskPhase::WaitingForAgent) { return; }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_seconds);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() { finish(&entry, QuickAskPhase::Failed, Some(error("quick_ask_agent_timeout", "Agent did not become ready"))); return; }
            let result = tokio::select! {
                _ = entry.notify.notified() => { if cancelled(&entry) { return; } continue; },
                result = adapter.wait_ready(&binding, &agent, remaining.as_millis().min(20_000) as u64) => result,
            };
            if cancelled(&entry) { return; }
            match result { Ok(true) => break, Ok(false) => continue, Err(e) => { finish(&entry, QuickAskPhase::Failed, Some(e)); return; } }
        }
    }
    // Herdr's idle state alone does not prove the harness is at its question
    // prompt (for example, onboarding can also appear idle). Require the
    // user to inspect the pane and explicitly confirm before dispatch.
    if !phase(&entry, QuickAskPhase::ReadyToSend) { return; }
    loop {
        entry.notify.notified().await;
        let should_send = { let mut op = entry.state.lock().unwrap(); if op.cancelled { set_phase(&mut op, QuickAskPhase::Cancelled); return; } op.status.phase == QuickAskPhase::Sending };
        if should_send { break; }
    }
    match adapter.prompt_once(&binding, &agent, &request.prompt).await {
        Ok(()) => finish(&entry, QuickAskPhase::Submitted, None),
        Err(PromptError::NotSent(e)) => finish(&entry, QuickAskPhase::Failed, Some(e)),
        Err(PromptError::Unknown(e)) => finish(&entry, QuickAskPhase::OutcomeUnknown, Some(e)),
    }
}

