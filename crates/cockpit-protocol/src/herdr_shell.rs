use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Metadata advertised by Herdr's identity-checked client-shell endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct HerdrShellState {
    pub prefix_bindings: Vec<String>,
    pub status: HerdrShellStatus,
    pub commands: Vec<HerdrCommand>,
    pub popup: Option<HerdrPopup>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HerdrShellStatus {
    Live,
    Connecting,
    Disconnected,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct HerdrCommand {
    pub command_id: String,
    pub binding_labels: Vec<String>,
    pub action: HerdrCommandAction,
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HerdrCommandAction {
    Shell,
    Pane,
    Popup,
    PluginAction,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct HerdrPopup {
    pub terminal_id: String,
    pub title: String,
    pub width: Option<HerdrPopupSize>,
    pub height: Option<HerdrPopupSize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HerdrPopupSize {
    Cells { value: u16 },
    Percent { value: u8 },
}
