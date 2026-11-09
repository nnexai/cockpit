use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_SCOPE_FRAME: usize = 256 * 1024;
pub const MAX_EVENT_FRAME: usize = 4096;
pub const MAX_ACK_FRAME: usize = 256;
pub const MAX_SAFE_SEQUENCE: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum LocalChangeScope {
    NotesContent { notes_id: String },
    NotesBinding { session_id: String, space_id: String },
    LibraryListing {},
    ContextFiles { session_id: String, viewer_id: String, binding_id: String, root_id: String, paths: Vec<String> },
    ContextIndex { session_id: String, viewer_id: String, binding_id: String, root_id: String },
    LibraryIndex { root_id: String },
    SpaceGit { session_id: String, server_instance: String },
    SpaceTags { session_id: String, server_instance: String },
    Orchestration { session_id: String, server_instance: String, state_store_id: String, root_id: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum LocalChangeTopic { Binding, Scratchpad, Todos, Decisions, Comments, Listing, FollowStatus, Files, Index, SpaceGit, SpaceTags, Orchestration }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum LocalChangeKind { Ready, Changed, Unavailable }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LocalChangeReason { pub code: String, pub message: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LocalChangeEvent {
    pub stream_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub kind: LocalChangeKind,
    pub topics: Vec<LocalChangeTopic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<LocalChangeReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LocalChangeAck {
    pub stream_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
}

fn id(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 256 || value.chars().any(|c| c.is_control()) { Err("invalid identifier") } else { Ok(()) }
}
fn frame<T: Serialize>(value: &T, max: usize) -> Result<(), &'static str> {
    if serde_json::to_vec(value).map_err(|_| "invalid frame")?.len() > max { Err("frame too large") } else { Ok(()) }
}
impl LocalChangeScope {
    pub fn validate(&self) -> Result<(), &'static str> {
        frame(self, MAX_SCOPE_FRAME)?;
        match self {
            Self::NotesContent { notes_id } => id(notes_id)?,
            Self::NotesBinding { session_id, space_id } => { id(session_id)?; id(space_id)?; },
            Self::LibraryListing {} => {},
            Self::LibraryIndex { root_id } => id(root_id)?,
            Self::SpaceGit { session_id, server_instance } | Self::SpaceTags { session_id, server_instance } => { id(session_id)?; id(server_instance)?; },
            Self::Orchestration { session_id, server_instance, state_store_id, root_id } => {
                id(session_id)?; id(server_instance)?;
                if state_store_id.len() != 64 || !state_store_id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) { return Err("invalid state store identity"); }
                if let Some(root) = root_id {
                    let bytes = root.as_bytes();
                    if bytes.len() != 36 || bytes.iter().enumerate().any(|(i, b)| if [8, 13, 18, 23].contains(&i) { *b != b'-' } else { !b.is_ascii_hexdigit() }) { return Err("invalid root identity"); }
                }
            },
            Self::ContextFiles { session_id, viewer_id, binding_id, root_id, paths } => {
                for v in [session_id, viewer_id, binding_id, root_id] { id(v)?; }
                if paths.len() > 128 { return Err("too many paths"); }
                let mut unique = HashSet::new();
                for p in paths {
                    if p.is_empty() || p.len() > 4096 || p.contains('\0') || p.contains('\\') || p.starts_with('/') || p.split('/').any(|part| part.is_empty() || part == "." || part == "..") || !unique.insert(p) { return Err("invalid or duplicate path"); }
                }
            },
            Self::ContextIndex { session_id, viewer_id, binding_id, root_id } => for v in [session_id, viewer_id, binding_id, root_id] { id(v)?; },
        }
        Ok(())
    }
}
impl LocalChangeEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        frame(self, MAX_EVENT_FRAME)?; id(&self.stream_id)?;
        if self.sequence == 0 || self.sequence > MAX_SAFE_SEQUENCE { return Err("invalid sequence"); }
        let mut unique = HashSet::new();
        if self.topics.len() > 12 || self.topics.iter().any(|t| !unique.insert(t)) { return Err("duplicate or excessive topics"); }
        match (&self.kind, &self.reason) {
            (LocalChangeKind::Unavailable, Some(reason)) if !reason.code.is_empty() && reason.code.len() <= 64 && !reason.message.is_empty() && reason.message.len() <= 512 && !reason.code.chars().any(char::is_control) && !reason.message.chars().any(char::is_control) => {},
            (LocalChangeKind::Ready | LocalChangeKind::Changed, None) => {},
            _ => return Err("invalid reason"),
        }
        Ok(())
    }
}
impl LocalChangeAck {
    pub fn validate(&self) -> Result<(), &'static str> {
        frame(self, MAX_ACK_FRAME)?; id(&self.stream_id)?;
        if self.sequence == 0 || self.sequence > MAX_SAFE_SEQUENCE { return Err("invalid sequence"); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_scopes_and_bounded_paths() {
        assert!(serde_json::from_str::<LocalChangeScope>(r#"{"kind":"library_listing","path":"/tmp"}"#).is_err());
        let scope = |paths| LocalChangeScope::ContextFiles { session_id:"s".into(), viewer_id:"v".into(), binding_id:"b".into(), root_id:"r".into(), paths };
        assert!(scope(vec!["a/../b".into()]).validate().is_err());
        assert!(scope(vec!["a".into(), "a".into()]).validate().is_err());
        assert!(scope(vec!["a".into(); 129]).validate().is_err());
        assert!(scope(vec!["a/b".into()]).validate().is_ok());
    }
    #[test]
    fn orchestration_scope_is_closed_and_validated() {
        let json = format!(r#"{{"kind":"orchestration","session_id":"s","server_instance":"v","state_store_id":"{}","root_id":null}}"#, "a".repeat(64));
        let scope: LocalChangeScope = serde_json::from_str(&json).unwrap();
        assert!(scope.validate().is_ok());
        assert!(serde_json::from_str::<LocalChangeScope>(&json.replace("\"root_id\":null", "\"root_id\":null,\"path\":\"/tmp\"")).is_err());
        let invalid = LocalChangeScope::Orchestration { session_id: "s".into(), server_instance: "v".into(), state_store_id: "x".repeat(64), root_id: None };
        assert!(invalid.validate().is_err());
        let invalid = LocalChangeScope::Orchestration { session_id: "s".into(), server_instance: "v".into(), state_store_id: "a".repeat(64), root_id: Some("not-a-uuid".into()) };
        assert!(invalid.validate().is_err());
        assert_eq!(serde_json::to_string(&LocalChangeTopic::Orchestration).unwrap(), "\"orchestration\"");
    }
    #[test]
    fn event_and_ack_constraints_are_checked() {
        let mut event = LocalChangeEvent { stream_id: "s".into(), sequence: 1, kind: LocalChangeKind::Changed, topics: vec![LocalChangeTopic::Files], reason: None };
        assert!(event.validate().is_ok());
        event.topics.push(LocalChangeTopic::Files);
        assert!(event.validate().is_err());
        event.topics.clear();
        event.reason = Some(LocalChangeReason { code: "failed".into(), message: "unavailable".into() });
        assert!(event.validate().is_err());
        event.kind = LocalChangeKind::Unavailable;
        assert!(event.validate().is_ok());
        event.reason.as_mut().unwrap().message = "x".repeat(513);
        assert!(event.validate().is_err());
        assert!(LocalChangeAck { stream_id: "s".into(), sequence: 0 }.validate().is_err());
        assert!(LocalChangeAck { stream_id: "s".into(), sequence: MAX_SAFE_SEQUENCE + 1 }.validate().is_err());
    }
}
