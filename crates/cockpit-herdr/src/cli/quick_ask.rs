use async_trait::async_trait;
use cockpit_core::{InspectionError, quick_ask::herdr::{AgentStart, LaunchError, PromptError, QuickAskAgent, QuickAskBinding, QuickAskHerdr, QuickAskPane}};
use serde_json::{Value, json};

use super::{HerdrCliAdapter, parse_snapshot};

fn error(code: &str, message: &str) -> InspectionError {
    InspectionError::new(code, message)
}

fn uncertain() -> InspectionError {
    error("herdr_outcome_unknown", "Herdr did not confirm the requested operation")
}

fn stale() -> InspectionError {
    error("stale_identity", "The quick ask target no longer matches the selected pane and agent")
}

fn matches_placement(value: &Value, binding: &QuickAskBinding, agent: &QuickAskAgent) -> bool {
    value["pane_id"].as_str() == Some(agent.pane.pane_id.as_str())
        && value["terminal_id"].as_str() == Some(agent.pane.terminal_id.as_str())
        && value["tab_id"].as_str() == Some(agent.pane.tab_id.as_str())
        && value["workspace_id"].as_str() == Some(binding.space_id.as_str())
        && value["agent"].as_str() == Some(agent.kind.as_str())
        && if agent.name.is_empty() { value["name"].is_null() } else { value["name"].as_str() == Some(agent.name.as_str()) }
}

// Herdr omits launch_pending when false; malformed or true values must not
// be treated as a ready agent.
fn launch_not_pending(agent: &Value) -> bool {
    matches!(agent.get("launch_pending"), None | Some(Value::Bool(false)))
}

fn append_initial_prompt(command: &mut Vec<String>, kind: &str, prompt: Option<&str>) -> Result<(), InspectionError> {
    let Some(prompt) = prompt else { return Ok(()); };
    match kind {
        "copilot" => command.push(format!("--interactive={prompt}")),
        "opencode" => command.push(format!("--prompt={prompt}")),
        "omp" | "pi" | "claude" | "codex"
            if !prompt.starts_with('@') && !prompt.starts_with('-') && prompt.bytes().any(|c| c == b' ' || c == b'\n') =>
                command.push(prompt.to_owned()),
        _ => return Err(error("quick_ask_invalid_agent", "Initial question arguments are unsupported for this input")),
    }
    Ok(())
}

fn conversation_identity(value: &Value) -> Result<Option<String>, InspectionError> {
    match value.get("agent_session") {
        None | Some(Value::Null) => Ok(None),
        Some(session @ Value::Object(_)) => {
            let kind = session["kind"].as_str().ok_or_else(stale)?;
            let source = session["source"].as_str().filter(|s| !s.is_empty()).ok_or_else(stale)?;
            let harness = session["agent"].as_str().filter(|s| !s.is_empty()).ok_or_else(stale)?;
            let reference = session["value"].as_str().filter(|s| !s.is_empty()).ok_or_else(stale)?;
            let journal_id = match kind {
                "id" => None,
                "path" if harness == "omp" && source == "herdr:omp" && std::path::Path::new(reference).is_absolute() => {
                    Some(cockpit_core::orchestration::session_identity::read_omp_session_file(std::path::Path::new(reference)).map_err(|_| stale())?)
                }
                _ => return Err(stale()),
            };
            serde_json::to_string(&(session, journal_id)).map(Some).map_err(|_| stale())
        }
        _ => Err(stale()),
    }
}

fn matches_agent(value: &Value, binding: &QuickAskBinding, agent: &QuickAskAgent) -> bool {
    matches_placement(value, binding, agent)
        && conversation_identity(value).ok().as_ref() == Some(&agent.session_identity)
}


impl HerdrCliAdapter {
    async fn quick_ask_snapshot(&self, b: &QuickAskBinding) -> Result<Value, InspectionError> {
        let (result, _) = self.socket_request_with_identity(
            &b.session_id, "session.snapshot", json!({}), Some(&b.endpoint_identity),
        ).await?;
        // Validate the full snapshot before using its identities.
        parse_snapshot(json!({"result": &result}), &b.session_id, &b.endpoint_identity)?;
        Ok(result)
    }

    async fn quick_ask_pane_gate(&self, b: &QuickAskBinding, pane: &QuickAskPane) -> Result<Value, InspectionError> {
        let result = self.quick_ask_snapshot(b).await?;
        let snapshot = &result["snapshot"];
        let panes = snapshot["panes"].as_array().ok_or_else(stale)?;
        let mut found = panes.iter().filter(|p| p["pane_id"].as_str() == Some(pane.pane_id.as_str()));
        let p = found.next().ok_or_else(stale)?;
        if found.next().is_some() || p["workspace_id"].as_str() != Some(b.space_id.as_str())
            || p["tab_id"].as_str() != Some(pane.tab_id.as_str())
            || p["terminal_id"].as_str() != Some(pane.terminal_id.as_str())
            || panes.iter().filter(|p| p["terminal_id"].as_str() == Some(pane.terminal_id.as_str())).count() != 1
        { return Err(stale()); }
        Ok(result)
    }

    async fn quick_ask_agent_gate(&self, b: &QuickAskBinding, agent: &QuickAskAgent) -> Result<(), InspectionError> {
        let snapshot = self.quick_ask_pane_gate(b, &agent.pane).await?;
        let agents = snapshot["snapshot"]["agents"].as_array().ok_or_else(stale)?;
        let mut found = agents.iter().filter(|item| item["pane_id"].as_str() == Some(agent.pane.pane_id.as_str()));
        let actual = found.next().ok_or_else(stale)?;
        if found.next().is_some() || !matches_agent(actual, b, agent) { return Err(stale()); }
        if !launch_not_pending(actual)
            || !matches!(actual["agent_status"].as_str(), Some("idle" | "done")) {
            return Err(error("quick_ask_agent_busy", "The agent is not ready for a question"));
        }
        Ok(())
    }
}

#[async_trait]
impl QuickAskHerdr for HerdrCliAdapter {
    async fn bind(&self, session_id: &str, server_instance: &str, space_id: &str) -> Result<QuickAskBinding, InspectionError> {
        let methods = self.project_methods(session_id).await?;
        for method in ["layout.apply", "agent.wait", "agent.prompt"] {
            if !methods.contains(method) { return Err(error("unsupported_capability", "Herdr does not support quick ask")); }
        }
        let (result, endpoint_identity) = self.socket_request_with_identity(session_id, "session.snapshot", json!({}), None).await?;
        let snapshot = parse_snapshot(json!({"result": result}), session_id, &endpoint_identity)?;
        if super::shell::server_instance(&endpoint_identity) != server_instance
            || snapshot.spaces.iter().filter(|space| space.id == space_id).count() != 1 {
            return Err(error("quick_ask_target_changed", "The selected Herdr server or Space changed"));
        }
        Ok(QuickAskBinding { session_id: session_id.into(), endpoint_identity, server_instance: server_instance.into(), space_id: space_id.into() })
    }

    async fn create_tab(&self, b: &QuickAskBinding, cwd: &str, label: &str, kind: &str, _name: &str, initial_prompt: Option<&str>) -> Result<QuickAskPane, LaunchError> {
        if !matches!(kind, "omp" | "pi" | "claude" | "copilot" | "codex" | "opencode") {
            return Err(LaunchError::NotSent(error("quick_ask_invalid_agent", "Unsupported agent")));
        }
        if !std::path::Path::new("/usr/bin/env").is_file() {
            return Err(LaunchError::NotSent(error("quick_ask_environment_unverified", "A clean environment launcher is unavailable")));
        }
        let before = self.quick_ask_snapshot(b).await.map_err(LaunchError::NotSent)?;
        let spaces = before["snapshot"]["workspaces"].as_array().ok_or_else(stale).map_err(LaunchError::NotSent)?;
        let tabs = before["snapshot"]["tabs"].as_array().ok_or_else(stale).map_err(LaunchError::NotSent)?;
        if spaces.iter().filter(|space| space["workspace_id"].as_str() == Some(b.space_id.as_str())).count() != 1
            || tabs.iter().any(|tab| tab["label"].as_str() == Some(label)) {
            return Err(LaunchError::NotSent(error("quick_ask_target_changed", "The Space changed or the quick ask tab label is already in use")));
        }
        // No shell is involved. The env executable discards the server's inherited
        // environment before starting the agent. Only non-secret environment values are copied.
        let mut command = vec!["/usr/bin/env".to_owned(), "-i".to_owned()];
        for key in ["HOME", "PATH", "TERM", "LANG", "LC_ALL", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "TMPDIR"] {
            if let Some(value) = std::env::var_os(key) {
                let value = value.into_string().map_err(|_| LaunchError::NotSent(error("quick_ask_environment_unverified", "A required environment value is not UTF-8")))?;
                command.push(format!("{key}={value}"));
            }
        }
        command.push(kind.to_owned());
        append_initial_prompt(&mut command, kind, initial_prompt).map_err(LaunchError::NotSent)?;
        let (result, _) = self.socket_request_with_identity(&b.session_id, "layout.apply",
            json!({"workspace_id": b.space_id, "tab_label": label, "focus": false,
                "root": {"type": "pane", "label": label, "cwd": cwd, "command": command}}),
            Some(&b.endpoint_identity)).await.map_err(|_| LaunchError::Unknown(uncertain()))?;
        let layout = &result["layout"];
        let root = &layout["root"];
        let tab_id = layout["tab_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| LaunchError::Unknown(uncertain()))?.to_owned();
        let receipt_pane = root["pane_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| LaunchError::Unknown(uncertain()))?;
        if result["type"].as_str() != Some("layout_apply")
            || layout["workspace_id"].as_str() != Some(b.space_id.as_str())
            || root["type"].as_str() != Some("pane") || root["label"].as_str() != Some(label)
            || root["command"] != json!(command) { return Err(LaunchError::Unknown(uncertain())); }
        let snapshot = self.quick_ask_snapshot(b).await.map_err(|_| LaunchError::Unknown(uncertain()))?;
        let panes = snapshot["snapshot"]["panes"].as_array().ok_or_else(|| LaunchError::Unknown(uncertain()))?;
        let mut found = panes.iter().filter(|p| p["tab_id"].as_str() == Some(tab_id.as_str()));
        let pane = found.next().ok_or_else(|| LaunchError::Unknown(uncertain()))?;
        if found.next().is_some() || pane["workspace_id"].as_str() != Some(b.space_id.as_str())
            || pane["pane_id"].as_str() != Some(receipt_pane) { return Err(LaunchError::Unknown(uncertain())); }
        let pane_id = pane["pane_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| LaunchError::Unknown(uncertain()))?;
        let terminal_id = pane["terminal_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| LaunchError::Unknown(uncertain()))?;
        if panes.iter().filter(|p| p["pane_id"].as_str() == Some(pane_id)).count() != 1
            || panes.iter().filter(|p| p["terminal_id"].as_str() == Some(terminal_id)).count() != 1 { return Err(LaunchError::Unknown(uncertain())); }
        Ok(QuickAskPane { tab_id, pane_id: pane_id.into(), terminal_id: terminal_id.into() })
    }


    async fn start_agent(&self, b: &QuickAskBinding, pane: &QuickAskPane, kind: &str, _name: &str) -> Result<AgentStart, InspectionError> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            self.quick_ask_pane_gate(b, pane).await?;
            let result = self.socket_request_with_identity(&b.session_id, "agent.wait",
                json!({"target": pane.pane_id, "until": ["idle", "done", "blocked"], "timeout_ms": 10000}),
                Some(&b.endpoint_identity)).await;
            match result {
                Ok((result, _)) if result["type"].as_str() == Some("agent_info") => {
                    let observed_name = match &result["agent"]["name"] {
                        Value::Null => String::new(),
                        Value::String(name) if !name.is_empty() => name.clone(),
                        _ => return Err(stale()),
                    };
                    let agent = QuickAskAgent { pane: pane.clone(), kind: kind.into(), name: observed_name,
                        session_identity: conversation_identity(&result["agent"])? };
                    if !matches_agent(&result["agent"], b, &agent)
                        || !launch_not_pending(&result["agent"]) { return Err(stale()); }
                    match result["agent"]["agent_status"].as_str() {
                        Some("idle" | "done") => return Ok(AgentStart::Ready(agent)),
                        Some("blocked") => return Ok(AgentStart::Blocked(agent)),
                        _ => return Err(stale()),
                    }
                }
                Err(e) if matches!(e.code.as_str(), "agent_wait_timeout" | "timeout" | "agent_not_running" | "agent_not_found") => {}
                Err(e) => return Err(error(&e.code, "Could not observe the quick ask agent")),
                _ => return Err(stale()),
            }
            if tokio::time::Instant::now() >= deadline { return Err(uncertain()); }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    async fn wait_ready(&self, b: &QuickAskBinding, agent: &QuickAskAgent, timeout_ms: u64) -> Result<bool, InspectionError> {
        self.quick_ask_pane_gate(b, &agent.pane).await?;
        let result = self.socket_request_with_identity(&b.session_id, "agent.wait",
            json!({"target": agent.pane.pane_id, "until": ["idle", "done"], "timeout_ms": timeout_ms}), Some(&b.endpoint_identity)).await;
        match result {
            Ok((result, _)) if result["type"].as_str() == Some("agent_info") && matches_agent(&result["agent"], b, agent)
                && matches!(result["agent"]["agent_status"].as_str(), Some("idle" | "done"))
                && launch_not_pending(&result["agent"]) => Ok(true),
            Err(e) if matches!(e.code.as_str(), "agent_wait_timeout" | "timeout" | "agent_not_running" | "agent_not_found") => Ok(false),
            Err(e) => Err(error(&e.code, "Could not observe the quick ask agent")),
            _ => Err(stale()),
        }
    }

    async fn prompt_once(&self, b: &QuickAskBinding, agent: &QuickAskAgent, text: &str) -> Result<(), PromptError> {
        self.quick_ask_agent_gate(b, agent).await.map_err(PromptError::NotSent)?;
        let result = self.socket_request_with_identity(&b.session_id, "agent.prompt",
            json!({"target": if agent.name.is_empty() { &agent.pane.pane_id } else { &agent.name }, "text": text}), Some(&b.endpoint_identity)).await;
        match result {
            Ok((result, _)) if result["type"].as_str() == Some("agent_prompted") && matches_placement(&result["agent"], b, agent) => Ok(()),
            Err(e) if e.code == "agent_blocked" => Err(PromptError::NotSent(error("agent_blocked", "The agent is blocked"))),
            _ => Err(PromptError::Unknown(uncertain())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_requires_every_occupant_identity() {
        let binding = QuickAskBinding {
            session_id: "session".into(), endpoint_identity: "endpoint".into(),
            server_instance: "instance".into(), space_id: "w1".into(),
        };
        let agent = QuickAskAgent {
            pane: QuickAskPane { tab_id: "t1".into(), pane_id: "p1".into(), terminal_id: "term1".into() },
            kind: "omp".into(), name: "ask-omp-a".into(), session_identity: None,
        };
        let valid = json!({"workspace_id":"w1", "tab_id":"t1", "pane_id":"p1",
            "terminal_id":"term1", "agent":"omp", "name":"ask-omp-a"});
        assert!(matches_agent(&valid, &binding, &agent));
        for field in ["workspace_id", "tab_id", "pane_id", "terminal_id", "agent", "name"] {
            let mut invalid = valid.clone();
            invalid[field] = json!("replacement");
            assert!(!matches_agent(&invalid, &binding, &agent), "{field}");
        }
    }

    #[test]
    fn conversation_change_fences_wait_and_prompt_gate() {
        let binding = QuickAskBinding {
            session_id: "session".into(), endpoint_identity: "endpoint".into(),
            server_instance: "instance".into(), space_id: "w1".into(),
        };
        let session = json!({"agent":"claude", "source":"herdr:claude", "kind":"id", "value":"one"});
        let agent = QuickAskAgent {
            pane: QuickAskPane { tab_id: "t1".into(), pane_id: "p1".into(), terminal_id: "term1".into() },
            kind: "claude".into(), name: String::new(),
            session_identity: conversation_identity(&json!({"agent_session":session})).unwrap(),
        };
        let mut observed = json!({"workspace_id":"w1", "tab_id":"t1", "pane_id":"p1",
            "terminal_id":"term1", "agent":"claude", "name":null, "agent_session":session});
        assert!(matches_agent(&observed, &binding, &agent));
        observed["agent_session"]["value"] = json!("two");
        assert!(!matches_agent(&observed, &binding, &agent));
        observed["agent_session"] = Value::Null;
        assert!(!matches_agent(&observed, &binding, &agent));
        let mut absent = agent;
        absent.session_identity = None;
        assert!(matches_agent(&observed, &binding, &absent));
        observed["agent_session"] = json!({"kind":"id", "value":"new"});
        assert!(!matches_agent(&observed, &binding, &absent));
    }

    #[test]
    fn omitted_launch_pending_is_not_pending_but_invalid_values_fail_closed() {
        assert!(launch_not_pending(&json!({"agent_status": "idle"})));
        assert!(launch_not_pending(&json!({"launch_pending": false})));
        assert!(!launch_not_pending(&json!({"launch_pending": true})));
        assert!(!launch_not_pending(&json!({"launch_pending": null})));
        assert!(!launch_not_pending(&json!({"launch_pending": "false"})));
    }

    #[test]
    fn positional_startup_prompt_requires_safe_shape_and_is_one_argument() {
        for kind in ["omp", "pi", "claude", "codex"] {
            for prompt in ["hello world", "line one\nline two", " ! command"] {
                let mut argv = vec![kind.to_owned()];
                append_initial_prompt(&mut argv, kind, Some(prompt)).unwrap();
                assert_eq!(argv, vec![kind.to_owned(), prompt.to_owned()]);
            }
            for prompt in ["update", "logout", "@file name", "-flag value", "update\u{a0}now"] {
                let mut argv = vec![kind.to_owned()];
                assert!(append_initial_prompt(&mut argv, kind, Some(prompt)).is_err());
                assert_eq!(argv, vec![kind.to_owned()]);
            }
        }
    }

}
