use super::*;
use async_trait::async_trait;
use herdr::{QuickAskBinding, QuickAskPane, QuickAskAgent};
use std::sync::atomic::{AtomicUsize, Ordering};

fn request(prompt: &str) -> QuickAskStartRequest {
    QuickAskStartRequest {
        request_id: "c498f483-04b1-4e36-a8d9-590a66b667fa".into(),
        server_instance: "server".into(), space_id: "space".into(), cwd: "/".into(),
        harness: QuickAskHarness::Omp, prompt: prompt.into(),
    }
}

#[test]
fn validates_multiline_without_changing_it() {
    for text in ["hello", "  /help", "!question", "@file", "é", "a\nb"] { assert!(validate(&request(text)).is_ok()); }
    for text in ["", "   ", "a\rb", "a\x1bb", "a\0b", "a\u{85}b"] { assert_eq!(validate(&request(text)).unwrap_err().code, "quick_ask_invalid_prompt"); }
    assert!(validate(&request(&"a".repeat(16384))).is_ok());
    assert!(validate(&request(&"a".repeat(16385))).is_err());
}

#[test]
fn redacts_question_from_debug() {
    assert!(!format!("{:?}", request("secret payload")).contains("secret payload"));
}

#[test]
fn terminal_delivery_invariants() {
    let mut op = operation();
    for (phase, delivery, cancellable) in [(QuickAskPhase::ReadyToSend, QuickAskDelivery::NotSent, true), (QuickAskPhase::Sending, QuickAskDelivery::NotSent, false), (QuickAskPhase::InitialQuestionSupplied, QuickAskDelivery::InitialQuestionSupplied, false), (QuickAskPhase::Submitted, QuickAskDelivery::Submitted, false), (QuickAskPhase::OutcomeUnknown, QuickAskDelivery::Unknown, false), (QuickAskPhase::Cancelled, QuickAskDelivery::NotSent, false)] {
        set_phase(&mut op, phase);
        assert_eq!(op.status.delivery, delivery);
        assert_eq!(op.status.cancellable, cancellable);
    }
}

fn operation() -> Operation {
    Operation { status: QuickAskStatus { request_id: "id".into(), session_id: "session".into(), server_instance: "server".into(), harness: QuickAskHarness::Pi, phase: QuickAskPhase::ReadyToSend, delivery: QuickAskDelivery::NotSent, cancellable: true, location: None, error: None, updated_at_ms: 0 }, fingerprint: [0; 32], cancelled: false, manual_only: false }
}

struct Fake { calls: AtomicUsize, gate: Option<Arc<Notify>>, blocked: bool, unknown: bool }
#[async_trait]
impl QuickAskHerdr for Fake {
    async fn bind(&self, session_id: &str, server_instance: &str, space_id: &str) -> Result<QuickAskBinding, InspectionError> { Ok(QuickAskBinding { session_id: session_id.into(), endpoint_identity: "endpoint".into(), server_instance: server_instance.into(), space_id: space_id.into() }) }
    async fn create_tab(&self, _: &QuickAskBinding, _: &str, _: &str, _: &str, _: &str, initial_prompt: Option<&str>) -> Result<QuickAskPane, LaunchError> { if let Some(gate) = &self.gate { gate.notified().await; } if let Some(text) = initial_prompt { assert_eq!(text, "literal question"); self.calls.fetch_add(1, Ordering::SeqCst); if self.unknown { return Err(LaunchError::Unknown(error("response_timeout", "untrusted"))); } } Ok(QuickAskPane { tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: "terminal".into() }) }
    async fn start_agent(&self, _: &QuickAskBinding, pane: &QuickAskPane, kind: &str, _: &str) -> Result<AgentStart, InspectionError> { let agent = QuickAskAgent { pane: pane.clone(), kind: kind.into(), name: "observed".into(), session_identity: Some("session".into()) }; Ok(if self.blocked { AgentStart::Blocked(agent) } else { AgentStart::Ready(agent) }) }
    async fn wait_ready(&self, _: &QuickAskBinding, _: &QuickAskAgent, _: u64) -> Result<bool, InspectionError> { Ok(true) }
    async fn prompt_once(&self, _: &QuickAskBinding, _: &QuickAskAgent, text: &str) -> Result<(), PromptError> { assert_eq!(text, "@literal question"); self.calls.fetch_add(1, Ordering::SeqCst); if self.unknown { Err(PromptError::Unknown(error("response_timeout", "untrusted"))) } else { Ok(()) } }
}
fn fake(blocked: bool, unknown: bool, gate: Option<Arc<Notify>>) -> Arc<Fake> { Arc::new(Fake { calls: AtomicUsize::new(0), gate, blocked, unknown }) }
async fn wait_phase(service: &QuickAskService, phase: QuickAskPhase) -> QuickAskStatus {
    tokio::time::timeout(Duration::from_secs(2), async { loop { let status = service.status("session", &request("literal question").request_id).await.unwrap(); if status.phase == phase { return status; } tokio::task::yield_now().await; } }).await.unwrap()
}

#[tokio::test]
async fn send_and_cancel_are_mutually_exclusive() {
    let service = QuickAskService::new(fake(false, false, None));
    let entry = Arc::new(Entry { state: Mutex::new(operation()), notify: Notify::new() });
    service.entries.lock().await.insert("id".into(), entry);
    let action = |action| QuickAskActionRequest { request_id: "id".into(), action };
    assert_eq!(service.action("session", action(QuickAskAction::Send)).await.unwrap().phase, QuickAskPhase::Sending);
    assert_eq!(service.action("session", action(QuickAskAction::Send)).await.unwrap_err().code, "quick_ask_invalid_action");
    assert_eq!(service.action("session", action(QuickAskAction::Cancel)).await.unwrap_err().code, "quick_ask_cancel_too_late");
    assert_eq!(service.status("other", "id").await.unwrap_err().code, "quick_ask_request_unknown");
}

#[tokio::test]
async fn duplicate_request_and_unknown_outcome_never_resend() {
    let adapter = fake(false, true, None);
    let service = QuickAskService::new(adapter.clone());
    let req = request("@literal question");
    service.start("session", req.clone()).await.unwrap();
    wait_phase(&service, QuickAskPhase::ReadyToSend).await;
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    service.action("session", QuickAskActionRequest { request_id: req.request_id.clone(), action: QuickAskAction::Send }).await.unwrap();
    let status = wait_phase(&service, QuickAskPhase::OutcomeUnknown).await;
    assert_eq!(status.delivery, QuickAskDelivery::Unknown);
    assert_eq!(service.start("session", req.clone()).await.unwrap().phase, QuickAskPhase::OutcomeUnknown);
    let mut changed = req; changed.prompt = "different".into();
    assert_eq!(service.start("session", changed).await.unwrap_err().code, "quick_ask_request_conflict");
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn blocked_agent_requires_explicit_send() {
    let adapter = fake(true, false, None);
    let service = QuickAskService::new(adapter.clone());
    let req = request("@literal question");
    service.start("session", req.clone()).await.unwrap();
    wait_phase(&service, QuickAskPhase::ReadyToSend).await;
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    service.action("session", QuickAskActionRequest { request_id: req.request_id, action: QuickAskAction::Send }).await.unwrap();
    wait_phase(&service, QuickAskPhase::Submitted).await;
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancel_during_creation_never_sends() {
    let gate = Arc::new(Notify::new());
    let adapter = fake(false, false, Some(gate.clone()));
    let service = QuickAskService::new(adapter.clone());
    let req = request("@literal question");
    service.start("session", req.clone()).await.unwrap();
    service.action("session", QuickAskActionRequest { request_id: req.request_id, action: QuickAskAction::Cancel }).await.unwrap();
    gate.notify_one();
    wait_phase(&service, QuickAskPhase::Cancelled).await;
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn initial_question_launch_is_single_and_reports_only_supply() {
    for harness in [QuickAskHarness::Copilot, QuickAskHarness::Opencode] {
        let adapter = fake(false, false, None);
        let service = QuickAskService::new(adapter.clone());
        let mut req = request("literal question");
        req.harness = harness;
        service.start("session", req.clone()).await.unwrap();
        let status = wait_phase(&service, QuickAskPhase::InitialQuestionSupplied).await;
        assert_eq!(status.delivery, QuickAskDelivery::InitialQuestionSupplied);
        assert!(!status.cancellable);
        service.start("session", req).await.unwrap();
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn uncertain_initial_launch_is_not_retried() {
    let adapter = fake(false, true, None);
    let service = QuickAskService::new(adapter.clone());
    let mut req = request("literal question");
    req.harness = QuickAskHarness::Copilot;
    service.start("session", req.clone()).await.unwrap();
    let status = wait_phase(&service, QuickAskPhase::OutcomeUnknown).await;
    assert_eq!(status.delivery, QuickAskDelivery::Unknown);
    service.start("session", req).await.unwrap();
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn startup_policy_avoids_command_like_positional_inputs() {
    for harness in [QuickAskHarness::Omp, QuickAskHarness::Pi, QuickAskHarness::Claude, QuickAskHarness::Codex] {
        let mut req = request("ordinary question\nsecond line");
        req.harness = harness;
        assert!(uses_initial_prompt(&req));
        for prompt in ["logout", "install", "word\u{feff}word", "word\u{a0}word", "@file", "@file\nquestion", "-option question"] {
            req.prompt = prompt.into();
            assert!(!uses_initial_prompt(&req), "{harness:?}: {prompt:?}");
        }
    }
    for harness in [QuickAskHarness::Copilot, QuickAskHarness::Opencode] {
        let mut req = request("@file\nquestion");
        req.harness = harness;
        assert!(uses_initial_prompt(&req));
    }
}

#[tokio::test]
async fn ambiguous_multiline_requires_manual_entry() {
    let adapter = fake(false, false, None);
    let service = QuickAskService::new(adapter.clone());
    let req = request("@file\nquestion");
    service.start("session", req.clone()).await.unwrap();
    wait_phase(&service, QuickAskPhase::ReadyToSend).await;
    let result = service.action("session", QuickAskActionRequest { request_id: req.request_id, action: QuickAskAction::Send }).await;
    assert_eq!(result.unwrap_err().code, "quick_ask_manual_multiline");
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
}
