use async_trait::async_trait;
use cockpit_core::{CommentPasteAdapter, InspectionError};
use cockpit_protocol::comment_paste::CommentPasteTarget;
use cockpit_protocol::v1::FocusKind;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::cli::{HerdrCliAdapter, parse_focus_result};

const MAX_PASTE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentSession {
    source: String,
    agent: String,
    kind: String,
    value: String,
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<&'a str, InspectionError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            InspectionError::new(
                "comments_paste_malformed_target",
                format!("{context}.{key} is required"),
            )
        })
}

fn optional_string<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn session(
    object: &serde_json::Map<String, Value>,
) -> Result<Option<AgentSession>, InspectionError> {
    let Some(value) = object.get("agent_session") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value.as_object().ok_or_else(|| {
        InspectionError::new(
            "comments_paste_malformed_target",
            "agent_session must be an object when present",
        )
    })?;
    Ok(Some(AgentSession {
        source: required_string(object, "source", "agent_session")?.to_owned(),
        agent: required_string(object, "agent", "agent_session")?.to_owned(),
        kind: required_string(object, "kind", "agent_session")?.to_owned(),
        value: required_string(object, "value", "agent_session")?.to_owned(),
    }))
}

fn fingerprint(terminal_id: &str, agent: &str, agent_session: Option<&AgentSession>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"cockpit-comment-paste-fingerprint-v1");
    for part in [terminal_id, agent] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    match agent_session {
        None => digest.update([0]),
        Some(session) => {
            digest.update([1]);
            for part in [
                &session.source,
                &session.agent,
                &session.kind,
                &session.value,
            ] {
                digest.update((part.len() as u64).to_be_bytes());
                digest.update(part.as_bytes());
            }
        }
    }
    format!("sha256:{:x}", digest.finalize())
}

fn targets_from_snapshot(
    value: Value,
    endpoint_identity: String,
    session_id: &str,
) -> Result<Vec<CommentPasteTarget>, InspectionError> {
    let result = value.as_object().ok_or_else(|| {
        InspectionError::new(
            "comments_paste_malformed_target",
            "session snapshot response must be an object",
        )
    })?;
    if required_string(result, "type", "session snapshot result")? != "session_snapshot" {
        return Err(InspectionError::new(
            "comments_paste_malformed_target",
            "session snapshot response has an unexpected type",
        ));
    }
    let snapshot = result
        .get("snapshot")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            InspectionError::new(
                "comments_paste_malformed_target",
                "session snapshot response omitted its snapshot",
            )
        })?;
    let agents = snapshot
        .get("agents")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InspectionError::new(
                "comments_paste_malformed_target",
                "session snapshot omitted agents",
            )
        })?;
    let mut agent_sessions = std::collections::BTreeMap::new();
    for raw in agents {
        let agent = raw.as_object().ok_or_else(|| {
            InspectionError::new(
                "comments_paste_malformed_target",
                "agent snapshot entry must be an object",
            )
        })?;
        let pane_id = required_string(agent, "pane_id", "agent snapshot")?;
        // Herdr reports a pane before detecting its agent. Display labels do
        // not establish a paste recipient; only the confirmed agent identity does.
        if agent.get("agent").is_none_or(Value::is_null) {
            continue;
        }
        let agent_name = required_string(agent, "agent", "agent snapshot")?;
        let session = match session(agent) {
            Ok(session) => session,
            // A malformed present session reference must never fall back to a
            // same-type agent identity.
            Err(_) => continue,
        };
        agent_sessions.insert(pane_id.to_owned(), (agent_name.to_owned(), session));
    }
    let panes = snapshot
        .get("panes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InspectionError::new(
                "comments_paste_malformed_target",
                "session snapshot omitted panes",
            )
        })?;
    let mut targets = Vec::new();
    for raw in panes {
        let pane = raw.as_object().ok_or_else(|| {
            InspectionError::new(
                "comments_paste_malformed_target",
                "pane snapshot entry must be an object",
            )
        })?;
        let pane_id = required_string(pane, "pane_id", "pane snapshot")?;
        let Some((confirmed_agent, agent_session)) = agent_sessions.get(pane_id) else {
            continue;
        };
        let Some(pane_agent) = optional_string(pane, "agent") else {
            continue;
        };
        if pane_agent != confirmed_agent {
            continue;
        }
        let pane_session = match session(pane) {
            Ok(session) => session,
            Err(_) => continue,
        };
        if let (Some(agent_session), Some(pane_session)) = (agent_session, &pane_session)
            && agent_session != pane_session
        {
            continue;
        }
        // A partially described agent can never become a paste target.
        let Some(terminal_id) = optional_string(pane, "terminal_id") else {
            continue;
        };
        let Some(workspace_id) = optional_string(pane, "workspace_id") else {
            continue;
        };
        let Some(tab_id) = optional_string(pane, "tab_id") else {
            continue;
        };
        targets.push(CommentPasteTarget {
            endpoint_identity: endpoint_identity.clone(),
            session_id: session_id.to_owned(),
            workspace_id: workspace_id.to_owned(),
            tab_id: tab_id.to_owned(),
            pane_id: pane_id.to_owned(),
            terminal_id: terminal_id.to_owned(),
            agent_label: pane_agent.to_owned(),
            agent_fingerprint: fingerprint(
                terminal_id,
                pane_agent,
                agent_session.as_ref().or(pane_session.as_ref()),
            ),
        });
    }
    Ok(targets)
}

fn confirm_focused_target(
    value: Value,
    endpoint_identity: String,
    target: &CommentPasteTarget,
) -> Result<(), InspectionError> {
    let snapshot = value
        .as_object()
        .and_then(|result| result.get("snapshot"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            InspectionError::new(
                "comments_paste_malformed_target",
                "session snapshot response omitted its snapshot",
            )
        })?;
    let focused =
        |key: &str, expected: &str| snapshot.get(key).and_then(Value::as_str) == Some(expected);
    if !focused("focused_workspace_id", &target.workspace_id)
        || !focused("focused_tab_id", &target.tab_id)
        || !focused("focused_pane_id", &target.pane_id)
    {
        return Err(InspectionError::new(
            "comments_paste_focus_lost",
            "Herdr snapshot no longer confirms the target workspace, tab, and pane focus",
        ));
    }
    let pane_focused = snapshot
        .get("panes")
        .and_then(Value::as_array)
        .and_then(|panes| {
            panes.iter().find(|pane| {
                pane.get("pane_id").and_then(Value::as_str) == Some(target.pane_id.as_str())
            })
        })
        .and_then(Value::as_object)
        .and_then(|pane| pane.get("focused"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !pane_focused {
        return Err(InspectionError::new(
            "comments_paste_focus_lost",
            "Herdr snapshot does not mark the target pane focused",
        ));
    }
    if !targets_from_snapshot(value, endpoint_identity, &target.session_id)?
        .iter()
        .any(|candidate| {
            candidate.endpoint_identity == target.endpoint_identity
                && candidate.session_id == target.session_id
                && candidate.workspace_id == target.workspace_id
                && candidate.tab_id == target.tab_id
                && candidate.pane_id == target.pane_id
                && candidate.terminal_id == target.terminal_id
                && candidate.agent_fingerprint == target.agent_fingerprint
        })
    {
        return Err(InspectionError::new(
            "comments_paste_target_mismatch",
            "Herdr snapshot no longer confirms the focused agent target",
        ));
    }
    Ok(())
}

fn validate_paste_ack(value: Value) -> Result<(), InspectionError> {
    let result = value.as_object().ok_or_else(|| {
        InspectionError::new(
            "comments_paste_malformed_ack",
            "Herdr paste acknowledgment must be an object; dispatch outcome is unknown",
        )
    })?;
    if result.get("type").and_then(Value::as_str) != Some("ok") {
        // pane.send_text has already been dispatched. An unexpected success
        // payload is not evidence that Herdr rejected the write.
        return Err(InspectionError::new(
            "comments_paste_malformed_ack",
            "Herdr paste acknowledgment was unexpected; dispatch outcome is unknown",
        ));
    }
    Ok(())
}

#[async_trait]
impl CommentPasteAdapter for HerdrCliAdapter {
    async fn comment_paste_targets(
        &self,
        session_id: &str,
    ) -> Result<Vec<CommentPasteTarget>, InspectionError> {
        let (result, endpoint_identity) = self
            .socket_request_with_identity(session_id, "session.snapshot", json!({}), None)
            .await?;
        targets_from_snapshot(result, endpoint_identity, session_id)
    }

    async fn focus_comment_paste_target(
        &self,
        target: &CommentPasteTarget,
    ) -> Result<(), InspectionError> {
        let (result, actual_identity) = self
            .socket_request_with_identity(
                &target.session_id,
                "agent.focus",
                json!({"target": target.pane_id}),
                Some(&target.endpoint_identity),
            )
            .await?;
        if actual_identity != target.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint changed before target focus",
            ));
        }
        parse_focus_result(result, FocusKind::Agent, &target.pane_id)?;
        Ok(())
    }

    async fn confirm_comment_paste_target_focus(
        &self,
        target: &CommentPasteTarget,
    ) -> Result<(), InspectionError> {
        let (result, endpoint_identity) = self
            .socket_request_with_identity(
                &target.session_id,
                "session.snapshot",
                json!({}),
                Some(&target.endpoint_identity),
            )
            .await?;
        if endpoint_identity != target.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint changed after target focus",
            ));
        }
        confirm_focused_target(result, endpoint_identity, target)
    }

    async fn send_comment_paste(
        &self,
        target: &CommentPasteTarget,
        framed_payload: &str,
    ) -> Result<(), InspectionError> {
        if framed_payload.as_bytes().len() > MAX_PASTE_BYTES {
            return Err(InspectionError::new(
                "comments_paste_input_bounded",
                "framed paste exceeds the 64 KiB Herdr limit",
            ));
        }
        if !framed_payload.starts_with("\u{1b}[200~") || !framed_payload.ends_with("\u{1b}[201~") {
            return Err(InspectionError::new(
                "comments_paste_framing",
                "paste adapter requires one complete bracketed-paste frame",
            ));
        }
        let (result, actual_identity) = self
            .socket_request_with_identity(
                &target.session_id,
                "pane.send_text",
                json!({"pane_id": target.pane_id, "text": framed_payload}),
                Some(&target.endpoint_identity),
            )
            .await?;
        if actual_identity != target.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint changed during paste dispatch",
            ));
        }
        validate_paste_ack(result)
    }
}

#[cfg(test)]
mod tests {
    use super::{confirm_focused_target, targets_from_snapshot, validate_paste_ack};
    use serde_json::json;

    fn snapshot(agent_session: serde_json::Value) -> serde_json::Value {
        json!({
            "type": "session_snapshot",
            "snapshot": {
                "focused_workspace_id": "w1",
                "focused_tab_id": "t1",
                "focused_pane_id": "w1:p1",
                "agents": [{
                    "pane_id": "w1:p1",
                    "agent": "codex",
                    "agent_session": agent_session,
                }],
                "panes": [{
                    "pane_id": "w1:p1",
                    "terminal_id": "terminal-1",
                    "workspace_id": "w1",
                    "tab_id": "t1",
                    "agent": "codex",
                    "focused": true,
                }],
            },
        })
    }

    #[test]
    fn unexpected_paste_ack_is_unknown_after_dispatch() {
        assert!(validate_paste_ack(json!({"type": "ok"})).is_ok());
        for value in [json!({"type": "rejected"}), json!({"type": "error"}), json!("ok")] {
            let error = validate_paste_ack(value).expect_err("unexpected ack must be unknown");
            assert_eq!(error.code, "comments_paste_malformed_ack");
            assert!(error.message.contains("unknown"));
        }
    }

    #[test]
    fn changed_native_agent_session_changes_the_target_fingerprint() {

        let first = targets_from_snapshot(
            snapshot(json!({"source":"herdr:codex","agent":"codex","kind":"id","value":"first"})),
            "endpoint".to_owned(),
            "session",
        )
        .expect("first target");
        let restarted = targets_from_snapshot(
            snapshot(json!({"source":"herdr:codex","agent":"codex","kind":"id","value":"second"})),
            "endpoint".to_owned(),
            "session",
        )
        .expect("restarted target");
        assert_eq!(first.len(), 1);
        assert_eq!(restarted.len(), 1);
        assert_ne!(first[0].agent_fingerprint, restarted[0].agent_fingerprint);
    }

    #[test]
    fn unidentified_agent_does_not_become_a_paste_target() {
        let mut raw = snapshot(serde_json::Value::Null);
        raw["snapshot"]["agents"][0]["agent"] = serde_json::Value::Null;
        raw["snapshot"]["agents"][0]["display_agent"] = json!("codex");
        raw["snapshot"]["agents"][0]["name"] = json!("codex");
        assert!(targets_from_snapshot(raw.clone(), "endpoint".to_owned(), "session")
            .expect("an unidentified agent is not a malformed snapshot")
            .is_empty());

        raw["snapshot"]["agents"][0]["agent"] = json!({"name":"codex"});
        assert!(targets_from_snapshot(raw.clone(), "endpoint".to_owned(), "session").is_err());

        raw["snapshot"]["agents"][0]["agent"] = json!("codex");
        assert_eq!(
            targets_from_snapshot(raw, "endpoint".to_owned(), "session")
                .expect("a confirmed matching agent")
                .len(),
            1
        );
    }

    #[test]
    fn missing_native_agent_session_uses_a_valid_fallback_identity() {
        let mut raw = snapshot(serde_json::Value::Null);
        raw["snapshot"]["agents"][0]
            .as_object_mut()
            .expect("agent object")
            .remove("agent_session");
        let targets = targets_from_snapshot(raw, "endpoint".to_owned(), "session")
            .expect("missing session is optional");
        assert_eq!(targets.len(), 1);
        assert!(targets[0].agent_fingerprint.starts_with("sha256:"));
    }

    #[test]
    fn malformed_present_agent_session_excludes_the_target() {
        let targets = targets_from_snapshot(
            snapshot(json!({"source":"herdr:codex","agent":"codex"})),
            "endpoint".to_owned(),
            "session",
        )
        .expect("malformed session rejects only this target");
        assert!(targets.is_empty());
    }

    #[test]
    fn post_focus_snapshot_requires_the_same_focused_pane() {
        let raw =
            snapshot(json!({"source":"herdr:codex","agent":"codex","kind":"id","value":"one"}));
        let target = targets_from_snapshot(raw.clone(), "endpoint".to_owned(), "session")
            .expect("target")[0]
            .clone();
        confirm_focused_target(raw, "endpoint".to_owned(), &target).expect("focused target");

        let mut moved =
            snapshot(json!({"source":"herdr:codex","agent":"codex","kind":"id","value":"one"}));
        moved["snapshot"]["focused_pane_id"] = json!("w1:p2");
        assert_eq!(
            confirm_focused_target(moved, "endpoint".to_owned(), &target)
                .expect_err("lost focus must reject")
                .code,
            "comments_paste_focus_lost",
        );
    }
}
