use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::orchestration::HandoffBinding;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceConnectRequest {
    pub session_id: String,
    pub sdp_offer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub selected_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project_scope: Option<HandoffBinding>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct VoiceConnectResponse {
    pub call_id: String,
    #[ts(type = "number")]
    pub generation: u64,
    pub sdp_answer: String,
    pub voice: String,
    pub liaison_root_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceEventsRequest {
    pub call_id: String,
    #[ts(type = "number")]
    pub generation: u64,
    #[ts(type = "number")]
    pub after_seq: u64,
    pub wait_ms: u32,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct VoiceEventsResponse {
    pub events: Vec<VoiceEvent>,
    #[ts(type = "number")]
    pub next_seq: u64,
    pub ended: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct VoiceControlRequest {
    pub call_id: String,
    #[ts(type = "number")]
    pub generation: u64,
    pub control: VoiceControl,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum VoiceControl {
    Mute { muted: bool },
    Stop,
}

/// A sequenced event belongs to exactly one call generation.
#[derive(Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct VoiceEvent {
    pub call_id: String,
    #[ts(type = "number")]
    pub generation: u64,
    #[ts(type = "number")]
    pub seq: u64,
    #[serde(flatten)]
    pub event: VoiceEventKind,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEventKind {
    Phase { phase: String },
    /// Text is cumulative for the identified turn, not a delta.
    Transcript { role: VoiceTranscriptRole, #[ts(type = "number")] turn: u64, text: String, r#final: bool },
    /// The authoritative handoff result, retained without inferring success.
    Handoff { #[ts(type = "unknown")] receipt: serde_json::Value },
    Ended { reason: Option<String>, error: Option<String> },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum VoiceTranscriptRole {
    User,
    Assistant,
}
