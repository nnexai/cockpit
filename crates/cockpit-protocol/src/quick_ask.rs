use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuickAskHarness { Omp, Pi, Claude, Copilot, Codex, Opencode }

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct QuickAskStartRequest {
    pub request_id: String,
    pub server_instance: String,
    pub space_id: String,
    pub cwd: String,
    pub harness: QuickAskHarness,
    pub prompt: String,
}


impl std::fmt::Debug for QuickAskStartRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuickAskStartRequest")
            .field("request_id", &self.request_id)
            .field("server_instance", &self.server_instance)
            .field("space_id", &self.space_id)
            .field("cwd", &self.cwd)
            .field("harness", &self.harness)
            .field("prompt", &"[redacted]")
            .finish()
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuickAskAction { Cancel, Send }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct QuickAskActionRequest { pub request_id: String, pub action: QuickAskAction }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuickAskPhase {
    CreatingTab, CheckingEnvironment, StartingAgent, WaitingForAgent, ReadyToSend,
    Sending, Submitted, InitialQuestionSupplied, Cancelled, Failed, OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuickAskDelivery { NotSent, Submitted, InitialQuestionSupplied, Unknown }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskLocation {
    pub space_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub tab_label: String,
    pub agent_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskFailure { pub code: String, pub message: String }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskStatus {
    pub request_id: String,
    pub session_id: String,
    pub server_instance: String,
    pub harness: QuickAskHarness,
    pub phase: QuickAskPhase,
    pub delivery: QuickAskDelivery,
    pub cancellable: bool,
    pub location: Option<QuickAskLocation>,
    pub error: Option<QuickAskFailure>,
    #[ts(type = "number")]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskAgentAvailability { pub harness: QuickAskHarness, pub executable: String, pub found: bool }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskAvailability { pub agents: Vec<QuickAskAgentAvailability> }
