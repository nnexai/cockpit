#![cfg(unix)]

use std::{os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

use cockpit_core::{
    InspectionError,
    orchestration::{
        Actor, OrchestrationService, ResolvedMutationInput,
        herdr::{AgentTabRequest, OrchestrationHerdr, PaneProcessInfo, RuntimeView},
    },
};
use cockpit_protocol::orchestration::{
    DispatchTarget, HandoffRecipient, InboxRequest, InboxState, OperatorOrigin,
    OrchestrationAction, OrchestrationActionResult, OrchestrationMutationRequest, RunLocation,
};
use uuid::Uuid;

const SESSION: &str = "inbox-close-fixture";

struct OfflineHerdr;

#[async_trait::async_trait]
impl OrchestrationHerdr for OfflineHerdr {
    async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> {
        Err(InspectionError::new("offline", "Fixture has no Herdr"))
    }
    async fn create_agent_tab(&self, _: &str, _: &AgentTabRequest) -> Result<RunLocation, InspectionError> {
        panic!("fixture must not launch an agent")
    }
    async fn pane_process_info(&self, _: &str, _: &str, _: &str) -> Result<PaneProcessInfo, InspectionError> {
        panic!("fixture must not inspect a pane")
    }
    async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
        panic!("fixture must not close a pane")
    }
}

struct Fixture {
    root: PathBuf,
    service: OrchestrationService,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-inbox-close-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(root.join("catalog")).unwrap();
        let directory = root.to_str().unwrap();
        std::fs::write(root.join("config.toml"), format!(
            "repository_roots = ['{directory}/catalog']\nstate_root = '{directory}/state'\ncache_root = '{directory}/cache'\nworktree_root = '{directory}/worktrees'\nlibrary_root = '{directory}/library'\nnotes_root = '{directory}/notes'\n"
        )).unwrap();
        let service = OrchestrationService::open_state_root(&root.join("state")).unwrap();
        Self { root, service }
    }

    fn apply(&self, action: OrchestrationAction) -> OrchestrationActionResult {
        self.service.mutate_resolved(
            &Actor::Operator(OperatorOrigin::Native),
            OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None, action },
            &ResolvedMutationInput::default(),
        ).unwrap().result
    }

    fn handoff(&self, root: &str, thread: &str) {
        self.apply(OrchestrationAction::ThreadHandoff {
            message_id: Uuid::new_v4().to_string(), thread_id: thread.into(), to_run_id: root.into(),
            text: "Investigate this request".into(), recipient: HandoffRecipient::Owner,
            images: Vec::new(), task_id: None, expected_task_revision: None, binding: None,
        });
    }

    fn cli(&self, operator: bool, thread: &str, session: &str) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cockpit"));
        command.env_clear()
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("xdg-config"))
            .env("XDG_DATA_HOME", self.root.join("xdg-data"))
            .env("XDG_STATE_HOME", self.root.join("xdg-state"))
            .env("XDG_CACHE_HOME", self.root.join("xdg-cache"))
            .arg("inbox")
            .args(["--config", self.root.join("config.toml").to_str().unwrap(),
                "--herdr-session", session, "--herdr", "/definitely/absent/herdr"]);
        if operator { command.arg("--operator"); }
        command.args(["close", "--thread", thread, "--json"]).output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

#[tokio::test]
async fn operator_cli_closes_exact_inbox_thread_without_accepting_or_notifying() {
    let fixture = Fixture::new();
    let OrchestrationActionResult::SupervisorStarted { start } = fixture.apply(
        OrchestrationAction::SupervisorStart {
            operation_id: Uuid::new_v4().to_string(),
            target: Some(DispatchTarget::ExistingSpace { workspace_id: "fixture-space".into() }),
            label: Some("Fixture supervisor".into()), selected_root_id: None,
        },
    ) else { panic!("expected supervisor start") };
    let thread = Uuid::new_v4().to_string();
    fixture.handoff(&start.root_id, &thread);
    let request = InboxRequest { session_id: SESSION.into() };
    let before = fixture.service.inbox(&OfflineHerdr, &request).await.unwrap();
    let item = before.threads.iter().find(|item| item.thread_id == thread).unwrap();
    assert_eq!(item.state, InboxState::Underway);
    let state_path = fixture.root.join("state/orchestration/state.json");
    let original = std::fs::read(&state_path).unwrap();
    let denied = fixture.cli(false, &thread, SESSION);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("credential_required"));
    assert_eq!(std::fs::read(&state_path).unwrap(), original);
    let foreign = fixture.cli(true, &thread, "foreign-session");
    assert!(!foreign.status.success());
    assert!(String::from_utf8_lossy(&foreign.stderr).contains("thread_close_invalid"));
    assert_eq!(std::fs::read(&state_path).unwrap(), original);
    let output = fixture.cli(true, &thread, SESSION);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let after = fixture.service.inbox(&OfflineHerdr, &request).await.unwrap();
    let item = after.threads.iter().find(|item| item.thread_id == thread).unwrap();
    assert_eq!(item.state, InboxState::Handled);
    assert!(item.closed_at.is_some());
    let closed = std::fs::read(&state_path).unwrap();
    let old: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let new: serde_json::Value = serde_json::from_slice(&closed).unwrap();
    for key in ["runs", "messages", "thread_links", "task_ownership"] {
        assert_eq!(old[key], new[key], "close must not change {key}");
    }
    let repeated = fixture.cli(true, &thread, SESSION);
    assert!(repeated.status.success(), "{}", String::from_utf8_lossy(&repeated.stderr));
    assert_eq!(std::fs::read(&state_path).unwrap(), closed);
    fixture.handoff(&start.root_id, &thread);
    let reopened = fixture.service.inbox(&OfflineHerdr, &request).await.unwrap();
    let item = reopened.threads.iter().find(|item| item.thread_id == thread).unwrap();
    assert_eq!(item.state, InboxState::Underway);
    assert!(item.closed_at.is_none());
}
