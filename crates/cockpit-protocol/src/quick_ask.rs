use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuickAskHarness { Omp, Pi, Claude, Copilot, Codex, Opencode }

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct QuickAskRequest {
    pub server_instance: String,
    pub space_id: String,
    pub cwd: String,
    pub harness: QuickAskHarness,
    pub prompt: String,
}

impl std::fmt::Debug for QuickAskRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuickAskRequest")
            .field("server_instance", &self.server_instance)
            .field("space_id", &self.space_id)
            .field("cwd", &self.cwd)
            .field("harness", &self.harness)
            .field("prompt", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskLocation {
    pub space_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub tab_label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskAgentAvailability { pub harness: QuickAskHarness, pub executable: String, pub found: bool }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct QuickAskAvailability { pub agents: Vec<QuickAskAgentAvailability> }
