use async_trait::async_trait;
use cockpit_core::{InspectionError, quick_ask::herdr::{QuickAskBinding, QuickAskHerdr}};
use cockpit_protocol::quick_ask::QuickAskLocation;
use serde_json::{Value, json};

use super::{HerdrCliAdapter, parse_snapshot};

fn error(code: &str, message: &str) -> InspectionError {
    InspectionError::new(code, message)
}

fn uncertain() -> InspectionError {
    error("herdr_outcome_unknown", "Herdr did not confirm the new tab")
}

fn stale() -> InspectionError {
    error("stale_identity", "The quick ask target no longer matches the selected Space")
}

fn append_initial_prompt(command: &mut Vec<String>, kind: &str, prompt: &str) -> Result<(), InspectionError> {
    match kind {
        "copilot" => command.push(format!("--interactive={prompt}")),
        "opencode" => command.push(format!("--prompt={prompt}")),
        "omp" | "pi" | "claude" | "codex" => {
            if prompt.starts_with('-') || !prompt.bytes().any(|c| c.is_ascii_whitespace()) {
                command.push(format!(" {prompt}"));
            } else {
                command.push(prompt.to_owned());
            }
        }
        _ => return Err(error("quick_ask_invalid_agent", "Unsupported agent")),
    }
    Ok(())
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

}

#[async_trait]
impl QuickAskHerdr for HerdrCliAdapter {
    async fn bind(&self, session_id: &str, server_instance: &str, space_id: &str) -> Result<QuickAskBinding, InspectionError> {
        let methods = self.project_methods(session_id).await?;
        if !methods.contains("layout.apply") {
            return Err(error("unsupported_capability", "Herdr does not support quick ask"));
        }
        let (result, endpoint_identity) = self.socket_request_with_identity(session_id, "session.snapshot", json!({}), None).await?;
        let snapshot = parse_snapshot(json!({"result": result}), session_id, &endpoint_identity)?;
        if super::shell::server_instance(&endpoint_identity) != server_instance
            || snapshot.spaces.iter().filter(|space| space.id == space_id).count() != 1 {
            return Err(error("quick_ask_target_changed", "The selected Herdr server or Space changed"));
        }
        Ok(QuickAskBinding { session_id: session_id.into(), endpoint_identity, server_instance: server_instance.into(), space_id: space_id.into() })
    }

    async fn create_tab(&self, b: &QuickAskBinding, cwd: &str, label: &str, kind: &str, prompt: &str, path: &str) -> Result<QuickAskLocation, InspectionError> {
        if !matches!(kind, "omp" | "pi" | "claude" | "copilot" | "codex" | "opencode") {
            return Err(error("quick_ask_invalid_agent", "Unsupported agent"));
        }
        if !std::path::Path::new("/usr/bin/env").is_file() {
            return Err(error("quick_ask_environment_unverified", "A clean environment launcher is unavailable"));
        }
        let before = self.quick_ask_snapshot(b).await?;
        let spaces = before["snapshot"]["workspaces"].as_array().ok_or_else(stale)?;
        let tabs = before["snapshot"]["tabs"].as_array().ok_or_else(stale)?;
        if spaces.iter().filter(|space| space["workspace_id"].as_str() == Some(b.space_id.as_str())).count() != 1 {
            return Err(error("quick_ask_target_changed", "The selected Space changed"));
        }
        // No shell is involved. The env executable discards the server's inherited
        // environment before starting the agent. Only non-secret environment values are copied.
        let mut command = vec!["/usr/bin/env".to_owned(), "-i".to_owned()];
        for key in ["HOME", "TERM", "LANG", "LC_ALL", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "TMPDIR"] {
            if let Some(value) = std::env::var_os(key) {
                let value = value.into_string().map_err(|_| error("quick_ask_environment_unverified", "A required environment value is not UTF-8"))?;
                command.push(format!("{key}={value}"));
            }
        }
        command.push(format!("PATH={path}"));
        command.push(kind.to_owned());
        append_initial_prompt(&mut command, kind, prompt)?;
        let (result, _) = self.socket_request_with_identity(&b.session_id, "layout.apply",
            json!({"workspace_id": b.space_id, "tab_label": label, "focus": false,
                "root": {"type": "pane", "label": label, "cwd": cwd, "command": command}}),
            Some(&b.endpoint_identity)).await.map_err(|_| uncertain())?;
        let layout = &result["layout"];
        let root = &layout["root"];
        let tab_id = layout["tab_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(uncertain)?.to_owned();
        if tabs.iter().any(|tab| tab["tab_id"].as_str() == Some(tab_id.as_str())) { return Err(uncertain()); }
        let receipt_pane = root["pane_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(uncertain)?;
        if result["type"].as_str() != Some("layout_apply")
            || layout["workspace_id"].as_str() != Some(b.space_id.as_str())
            || root["type"].as_str() != Some("pane") || root["label"].as_str() != Some(label)
            || root["command"] != json!(command) { return Err(uncertain()); }
        let snapshot = self.quick_ask_snapshot(b).await.map_err(|_| uncertain())?;
        let panes = snapshot["snapshot"]["panes"].as_array().ok_or_else(uncertain)?;
        let mut found = panes.iter().filter(|p| p["tab_id"].as_str() == Some(tab_id.as_str()));
        let pane = found.next().ok_or_else(uncertain)?;
        if found.next().is_some() || pane["workspace_id"].as_str() != Some(b.space_id.as_str())
            || pane["pane_id"].as_str() != Some(receipt_pane) { return Err(uncertain()); }
        let pane_id = pane["pane_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(uncertain)?;
        let terminal_id = pane["terminal_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(uncertain)?;
        if panes.iter().filter(|p| p["pane_id"].as_str() == Some(pane_id)).count() != 1
            || panes.iter().filter(|p| p["terminal_id"].as_str() == Some(terminal_id)).count() != 1 { return Err(uncertain()); }
        Ok(QuickAskLocation { space_id: b.space_id.clone(), tab_id, pane_id: pane_id.into(), terminal_id: terminal_id.into(), tab_label: label.into() })
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_prompt_is_one_argument_for_each_harness() {
        for kind in ["omp", "pi", "claude", "codex"] {
            for (prompt, expected) in [
                ("hello world", "hello world"),
                ("line one\nline two", "line one\nline two"),
                ("@file name", "@file name"),
                ("@file", " @file"),
                ("-flag value", " -flag value"),
                ("update", " update"),
                ("update\u{a0}now", " update\u{a0}now"),
                ("a\tb", "a\tb"),
            ] {
                let mut argv = vec![kind.to_owned()];
                append_initial_prompt(&mut argv, kind, prompt).unwrap();
                assert_eq!(argv, vec![kind.to_owned(), expected.to_owned()]);
            }
        }
        for (kind, prefix) in [("copilot", "--interactive="), ("opencode", "--prompt=")] {
            let mut argv = vec![kind.to_owned()];
            append_initial_prompt(&mut argv, kind, "@file -flag").unwrap();
            assert_eq!(argv, vec![kind.to_owned(), format!("{prefix}@file -flag")]);
        }
        let mut argv = Vec::new();
        assert!(append_initial_prompt(&mut argv, "unknown", "question").is_err());
        assert!(argv.is_empty());
    }

}
