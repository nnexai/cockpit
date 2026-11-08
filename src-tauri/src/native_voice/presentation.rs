use cockpit_protocol::{orchestration::HandoffBinding, voice::{VoiceEventKind, VoiceTranscriptRole}};
use serde::Serialize;
use serde_json::Value;

const MAX_TEXT: usize = 1024 * 1024;
const MAX_ITEMS: usize = 256;

#[derive(Clone, Serialize)]
pub struct NativeVoiceContext {
    pub session_id: String,
    pub selected_root_id: Option<String>,
    pub project_scope: Option<HandoffBinding>,
}

#[derive(Clone, Serialize)]
pub struct NativeVoiceTranscript {
    pub role: VoiceTranscriptRole,
    pub turn: u64,
    pub text: String,
    pub r#final: bool,
}

#[derive(Clone, Serialize)]
pub struct NativeVoicePresentation {
    pub view_generation: u64,
    pub revision: u64,
    pub status: String,
    pub phase: Option<String>,
    pub muted: bool,
    pub context: Option<NativeVoiceContext>,
    pub transcript: Vec<NativeVoiceTranscript>,
    pub handoffs: Vec<Value>,
    pub error: Option<String>,
    pub end_reason: Option<String>,
    pub earlier_text_trimmed: bool,
}

impl Default for NativeVoicePresentation {
    fn default() -> Self {
        Self { view_generation: 0, revision: 0, status: "idle".into(), phase: None, muted: false, context: None, transcript: Vec::new(), handoffs: Vec::new(), error: None, end_reason: None, earlier_text_trimmed: false }
    }
}

impl NativeVoicePresentation {
    pub(crate) fn apply(&mut self, event: VoiceEventKind) -> Result<bool, String> {
        match event {
            VoiceEventKind::Phase { phase } => {
                if phase.len() > 4096 { return Err("Voice phase exceeds presentation limit".into()); }
                self.phase = Some(phase);
            }
            VoiceEventKind::Transcript { role, turn, text, r#final } => {
                if text.len() > MAX_TEXT { return Err("Voice transcript exceeds presentation limit".into()); }
                if let Some(item) = self.transcript.iter_mut().find(|item| item.turn == turn && item.role == role) {
                    if item.r#final { return Ok(false); }
                    item.text = text;
                    item.r#final = r#final;
                } else {
                    self.transcript.push(NativeVoiceTranscript { role, turn, text, r#final });
                }
                while self.transcript.len() > MAX_ITEMS || self.transcript.iter().map(|item| item.text.len()).sum::<usize>() > MAX_TEXT {
                    let Some(index) = self.transcript.iter().position(|item| item.r#final) else { return Err("Active voice transcript exceeds presentation limit".into()); };
                    self.transcript.remove(index);
                    self.earlier_text_trimmed = true;
                }
            }
            VoiceEventKind::Handoff { receipt } => {
                if self.handoffs.len() >= MAX_ITEMS || serde_json::to_vec(&receipt).map_or(true, |v| v.len() > MAX_TEXT) || self.handoffs.iter().map(|r| serde_json::to_vec(r).map_or(MAX_TEXT, |v| v.len())).sum::<usize>() + serde_json::to_vec(&receipt).map_or(MAX_TEXT, |v| v.len()) > MAX_TEXT {
                    return Err("Voice handoff receipts exceed presentation limit".into());
                }
                self.handoffs.push(receipt);
            }
            VoiceEventKind::Ended { reason, error } => {
                self.end_reason = reason;
                self.error = error.map(|_| "The remote voice session reported an error".into());
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn transcript(role: VoiceTranscriptRole, turn: u64, text: &str, final_: bool) -> VoiceEventKind {
        VoiceEventKind::Transcript { role, turn, text: text.into(), r#final: final_ }
    }

    #[test]
    fn finalized_turn_does_not_regress() {
        let mut state = NativeVoicePresentation::default();
        state.apply(transcript(VoiceTranscriptRole::User, 1, "par", false)).unwrap();
        state.apply(transcript(VoiceTranscriptRole::User, 1, "complete", true)).unwrap();
        state.apply(transcript(VoiceTranscriptRole::User, 1, "late partial", false)).unwrap();
        state.apply(transcript(VoiceTranscriptRole::User, 1, "late final", true)).unwrap();
        assert_eq!(state.transcript.len(), 1);
        assert_eq!(state.transcript[0].text, "complete");
        assert!(state.transcript[0].r#final);
    }

    #[test]
    fn transcript_keys_include_role_and_turn() {
        let mut state = NativeVoicePresentation::default();
        state.apply(transcript(VoiceTranscriptRole::User, 1, "user", true)).unwrap();
        state.apply(transcript(VoiceTranscriptRole::Assistant, 1, "assistant", false)).unwrap();
        state.apply(transcript(VoiceTranscriptRole::User, 2, "next", false)).unwrap();
        state.apply(transcript(VoiceTranscriptRole::Assistant, 1, "assistant complete", true)).unwrap();
        assert_eq!(state.transcript.len(), 3);
        assert_eq!(state.transcript[0].text, "user");
        assert_eq!(state.transcript[1].text, "assistant complete");
        assert!(state.transcript[1].r#final);
        assert_eq!(state.transcript[2].text, "next");
        assert!(!state.transcript[2].r#final);
    }

    #[test]
    fn receipts_are_bounded_without_discarding_prior_outcomes() {
        let mut state = NativeVoicePresentation::default();
        for i in 0..MAX_ITEMS {
            state.apply(VoiceEventKind::Handoff { receipt: json!({"status": "unknown", "turn": i}) }).unwrap();
        }
        assert!(state.apply(VoiceEventKind::Handoff { receipt: json!({"status": "accepted"}) }).is_err());
        assert_eq!(state.handoffs.len(), MAX_ITEMS);
        assert_eq!(state.handoffs[0]["status"], "unknown");
    }
}
