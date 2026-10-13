use std::{
    collections::BTreeMap,
    path::Path,
};

use async_trait::async_trait;
use cockpit_core::{
    InspectionError,
    orchestration::herdr::{
        AgentPromptRequest, AgentTabRequest, DirectAgentTabRequest, OrchestrationHerdr,
        PaneProcessInfo, RuntimePane, RuntimeView, RuntimeWorkspace,
    },
};
use cockpit_core::orchestration::session_identity::read_omp_session_file;
use cockpit_protocol::orchestration::RunLocation;
use serde_json::{Value, json};

use super::{HerdrCliAdapter, object, parse_snapshot, required_string};

fn unknown(error: InspectionError) -> InspectionError {
    if matches!(
        error.code.as_str(),
        "request_outcome_unknown" | "malformed_json" | "malformed_response" | "response_timeout" | "session_mismatch"
            // Stock layout.apply can fail after spawning the new PTY command.
            // Its rollback acknowledgement is not proof that no writer ran.
            | "layout_apply_failed"
    ) {
        InspectionError::new("herdr_outcome_unknown", error.message)
    } else {
        error
    }
}

fn parse_pane_process_info(
    result: &Value,
    pane_id: &str,
) -> Result<PaneProcessInfo, InspectionError> {
    let response = object(result, "pane.process_info")?;
    if required_string(response, "type", "pane.process_info")? != "pane_process_info" {
        return Err(InspectionError::new("malformed_response", "Unexpected process-info response type"));
    }
    let info = response.get("process_info").ok_or_else(|| {
        InspectionError::new("malformed_response", "process_info is required")
    })?;
    let info = object(info, "process_info")?;
    let actual_pane_id = required_string(info, "pane_id", "process_info")?;
    if actual_pane_id != pane_id {
        return Err(InspectionError::new(
            "malformed_response",
            "process_info belongs to a different pane",
        ));
    }
    let optional_pid = |key: &str| -> Result<Option<u32>, InspectionError> {
        match info.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => parse_process_pid(value, key).map(Some),
        }
    };
    let processes = match info.get("foreground_processes") {
        None => Vec::new(),
        Some(Value::Array(processes)) => processes.iter().map(|value| {
            let process = object(value, "foreground process")?;
            let pid = process.get("pid").ok_or_else(|| {
                InspectionError::new("malformed_response", "foreground process PID is required")
            })?;
            Ok((
                parse_process_pid(pid, "foreground process PID")?,
                required_string(process, "name", "foreground process")?,
            ))
        }).collect::<Result<Vec<_>, InspectionError>>()?,
        Some(_) => return Err(InspectionError::new(
            "malformed_response",
            "foreground_processes must be an array",
        )),
    };
    Ok(PaneProcessInfo {
        pane_id: actual_pane_id,
        shell_pid: optional_pid("shell_pid")?,
        foreground_pgid: optional_pid("foreground_process_group_id")?,
        processes,
        shell_identity: None,
    })
}

fn parse_process_pid(value: &Value, field: &str) -> Result<u32, InspectionError> {
    value.as_u64().and_then(|pid| u32::try_from(pid).ok()).ok_or_else(|| {
        InspectionError::new("malformed_response", format!("{field} must be a uint32"))
    })
}

fn validate_close_receipt(result: &Value) -> Result<(), InspectionError> {
    // Herdr v0.9.3 handle_pane_close returns ResponseResult::Ok; any other
    // receipt is uncertain, not a refusal or proof that the pane disappeared.
    let response = object(result, "pane.close").map_err(unknown)?;
    if required_string(response, "type", "pane.close").ok().as_deref() != Some("ok") {
        return Err(InspectionError::new(
            "herdr_outcome_unknown",
            "pane.close returned no close acknowledgement",
        ));
    }
    Ok(())
}

fn validate_agent_tab_request(request: &AgentTabRequest) -> Result<(), InspectionError> {
    validate_tab_request(&request.cwd, &request.label, &request.launch_tag, &request.env)
}

fn validate_tab_request(
    cwd: &str,
    label: &str,
    launch_tag: &str,
    env: &BTreeMap<String, String>,
) -> Result<(), InspectionError> {
    let ordinary_terminal = launch_tag.is_empty()
        && label == "Working terminal"
        && env.is_empty();
    let tagged_launch = !launch_tag.is_empty() && label == launch_tag;
    if !Path::new(cwd).is_absolute() || !(ordinary_terminal || tagged_launch) {
        return Err(InspectionError::new(
            "invalid_launch_request",
            "Tab requires an absolute cwd and either an exact launch tag or an ordinary working terminal without launch environment",
        ));
    }
    Ok(())
}

fn direct_agent_tab_params(request: &DirectAgentTabRequest) -> Result<Value, InspectionError> {
    validate_tab_request(&request.cwd, &request.label, &request.launch_tag, &request.env)?;
    if request.launch_tag.is_empty()
        || request.command.first().is_none_or(|program| program.trim().is_empty()
            || program.starts_with('-') || program.len() > 4096 || program.chars().any(char::is_control))
        || request.command.len() > 19 // executable, 16 configured args, optional session-directory pair
        || request.command.iter().any(|arg| arg.len() > 4096 || arg.chars().any(char::is_control))
    {
        return Err(InspectionError::new(
            "invalid_launch_request",
            "Direct agent tab requires an exact launch tag and bounded literal executable argv",
        ));
    }
    let env = launch_environment(&request.env)?;
    // Stock Herdr v0.9.3 layout.apply: omitting tab_id creates a NEW tab;
    // omitting pane_id creates a NEW PTY rather than replacing a live pane.
    Ok(json!({
        "workspace_id": request.workspace_id,
        "tab_label": request.label,
        "focus": false,
        "root": {
            "type": "pane",
            "label": request.label,
            "cwd": request.cwd,
            "command": request.command,
            "env": env,
        },
    }))
}

fn direct_launch_receipt<'a>(
    result: &'a Value,
    request: &DirectAgentTabRequest,
) -> Result<(&'a str, &'a str), InspectionError> {
    // Herdr v0.9.3 schema/response.rs: LayoutApply { layout }, serialized
    // as type=layout_apply; schema/panes.rs: LayoutDescription / LayoutPane.
    let layout = &result["layout"];
    let root = &layout["root"];
    let tab_id = layout["tab_id"].as_str().filter(|id| !id.is_empty());
    let pane_id = root["pane_id"].as_str().filter(|id| !id.is_empty());
    let command_matches = root["command"].as_array().is_some_and(|argv| {
        argv.len() == request.command.len()
            && argv.iter().zip(&request.command).all(|(actual, expected)| {
                actual.as_str() == Some(expected.as_str())
            })
    });
    if result["type"].as_str() != Some("layout_apply")
        || layout["workspace_id"].as_str() != Some(request.workspace_id.as_str())
        || root["type"].as_str() != Some("pane")
        || root["label"].as_str() != Some(request.launch_tag.as_str())
        || tab_id.is_none()
        || pane_id.is_none()
        || layout["focused_pane_id"].as_str() != pane_id
        || !command_matches
    {
        return Err(InspectionError::new(
            "herdr_outcome_unknown",
            "layout.apply receipt does not match the exact direct launch workspace, tab, pane, tag and argv",
        ));
    }
    Ok((tab_id.unwrap(), pane_id.unwrap()))
}

fn direct_launch_location(
    session_id: &str,
    request: &DirectAgentTabRequest,
    tab_id: &str,
    pane_id: &str,
    before: &RuntimeView,
    runtime: &RuntimeView,
) -> Result<RunLocation, InspectionError> {
    let mut tab_panes = runtime.panes.iter().filter(|pane| pane.tab_id == tab_id);
    let pane = tab_panes.next();
    let terminal = pane.and_then(|pane| pane.terminal_id.as_deref()).filter(|id| !id.is_empty());
    if before.endpoint_identity != request.endpoint_identity
        || runtime.endpoint_identity != request.endpoint_identity
        || before.boot_id != runtime.boot_id
        || runtime.workspaces.iter().filter(|space| space.workspace_id == request.workspace_id).count() != 1
        || tab_panes.next().is_some()
        || pane.is_none_or(|pane| {
            pane.workspace_id != request.workspace_id
                || pane.pane_id != pane_id
                || pane.tab_label != request.launch_tag
        })
        || terminal.is_none()
        || runtime.panes.iter().filter(|pane| pane.pane_id == pane_id).count() != 1
        || runtime.panes.iter().filter(|pane| pane.tab_label == request.launch_tag).count() != 1
        || runtime.panes.iter().filter(|pane| pane.terminal_id.as_deref() == terminal).count() != 1
        || before.panes.iter().any(|pane| {
            pane.tab_id == tab_id || pane.pane_id == pane_id || pane.terminal_id.as_deref() == terminal
        })
    {
        return Err(InspectionError::new(
            "herdr_outcome_unknown",
            "Direct launch receipt lacks a fresh exact, unique tagged tab/pane/terminal on the pinned endpoint",
        ));
    }
    let pane = pane.unwrap();
    Ok(RunLocation {
        endpoint_identity: runtime.endpoint_identity.clone(),
        session_id: session_id.into(),
        workspace_id: request.workspace_id.clone(),
        tab_id: tab_id.into(),
        pane_id: pane_id.into(),
        launch_tag: request.launch_tag.clone(),
        boot_id: runtime.boot_id.clone(),
        terminal_id: pane.terminal_id.clone(),
        native_session_id: pane.native_session_id.clone(),
    })
}



fn launch_environment(env: &BTreeMap<String, String>) -> Result<Value, InspectionError> {
    if env.len() > 24 {
        return Err(InspectionError::new(
            "invalid_environment",
            "Too many launch environment entries",
        ));
    }
    let mut result = serde_json::Map::new();
    for (key, value) in env {
        let allowed = matches!(
            key.as_str(),
            "COCKPIT_RUN_ID"
                | "COCKPIT_RUN_ATTEMPT"
                | "COCKPIT_ROOT_ID"
                | "COCKPIT_LAUNCH_TAG"
                | "COCKPIT_LIBRARY_ROOT"
                | "COCKPIT_WORKSPACE_ID"
                | "COCKPIT_REPOSITORY_KEY"
                | "COCKPIT_ARTIFACT_URL"
                | "COCKPIT_CONFIG_PATH"
                | "COCKPIT_HARNESS"
                | "COCKPIT_HERDR_SESSION"
                | "COCKPIT_HERDR_SOCKET"
                | "COCKPIT_HERDR_EXECUTABLE"
                | "COCKPIT_CLI_PATH"
                | "COCKPIT_STATE_ROOT"
                | "COCKPIT_JOB_CREDENTIAL_FILE"
                | "COCKPIT_ATTACHMENT_ID"
                | "COCKPIT_ATTACHMENT_EPOCH"
                | "COCKPIT_CACHE_ROOT"
                | "COCKPIT_WORKTREE_ROOT"
                | "COCKPIT_REPOSITORY_ROOTS"
        );
        if !allowed || value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
        {
            return Err(InspectionError::new(
                "invalid_environment",
                "Launch environment contains an unsupported key or value",
            ));
        }
        result.insert(key.clone(), Value::String(value.clone()));
    }
    Ok(Value::Object(result))
}

fn validate_prompt_target(runtime: &RuntimeView, request: &AgentPromptRequest) -> Result<(), InspectionError> {
    if request.text.trim().is_empty() || request.text.len() > 4096
        || request.text.chars().any(char::is_control)
    {
        return Err(InspectionError::new("invalid_prompt_request", "Prompt must be bounded printable text"));
    }
    let location = &request.location;
    let terminal = location.terminal_id.as_deref().filter(|id| !id.is_empty());
    let mut panes = runtime.panes.iter().filter(|pane| pane.pane_id == location.pane_id);
    let pane = panes.next().ok_or_else(|| InspectionError::new("stale_identity", "Prompt pane is absent"))?;
    if runtime.endpoint_identity != location.endpoint_identity || panes.next().is_some()
        || terminal.is_none() || pane.terminal_id.as_deref() != terminal
        || runtime.panes.iter().filter(|pane| pane.terminal_id.as_deref() == terminal).count() != 1
        || pane.workspace_id != location.workspace_id || pane.tab_id != location.tab_id
        || location.launch_tag.is_empty() || pane.tab_label != location.launch_tag
        || location.boot_id.as_ref().is_some_and(|id| runtime.boot_id.as_ref() != Some(id))
        || location.native_session_id.as_ref().is_some_and(|id| pane.native_session_id.as_ref() != Some(id))
        || request.native_session_file.as_ref().is_some_and(|path| pane.native_session_file.as_ref() != Some(path))
    {
        return Err(InspectionError::new("stale_identity", "Prompt target no longer matches the owned launch placement and observed occupant"));
    }
    if pane.agent_kind.as_deref().is_none_or(str::is_empty) {
        return Err(InspectionError::new("herdr_prompt_unsupported", "Herdr has not observed a supported agent at this target"));
    }
    if request.harness_kind.is_empty() || pane.agent_kind.as_deref() != Some(request.harness_kind.as_str()) {
        return Err(InspectionError::new("stale_identity", "Prompt target harness changed"));
    }
    if pane.launch_pending
        || !matches!(pane.agent_status.as_deref(), Some("idle" | "done"))
    {
        return Err(InspectionError::new("herdr_prompt_deferred", "Prompt target is pending, busy or blocked"));
    }
    Ok(())
}

fn observed_omp_session_path(agent: &Value) -> Option<&str> {
    let session = agent.get("agent_session")?;
    let value = session.get("value")?.as_str()?;
    (agent["agent"].as_str() == Some("omp")
        && session["agent"].as_str() == Some("omp")
        && session["source"].as_str() == Some("herdr:omp")
        && session["kind"].as_str() == Some("path")
        && Path::new(value).is_absolute()
        && !value.chars().any(char::is_control))
        .then_some(value)
}

fn observed_session_id(agent: &Value, omp_path: Option<&str>) -> Option<String> {
    let session = agent.get("agent_session")?;
    match session.get("kind")?.as_str()? {
        "id" => session.get("value")?.as_str().map(str::to_owned),
        // Physical observations are optional evidence, not registration authority.
        "path" => read_omp_session_file(Path::new(omp_path?)).ok(),
        _ => None,
    }
}

fn validate_prompt_receipt(result: &Value, request: &AgentPromptRequest) -> Result<(), InspectionError> {
    let location = &request.location;
    let agent = &result["agent"];
    let omp_path = observed_omp_session_path(agent);
    if result["type"].as_str() != Some("agent_prompted")
        || agent["pane_id"].as_str() != Some(location.pane_id.as_str())
        || agent["terminal_id"].as_str() != location.terminal_id.as_deref()
        || agent["workspace_id"].as_str() != Some(location.workspace_id.as_str())
        || agent["tab_id"].as_str() != Some(location.tab_id.as_str())
        || agent["agent"].as_str() != Some(request.harness_kind.as_str())
        || location.native_session_id.as_ref().is_some_and(|id|
            observed_session_id(agent, omp_path).as_ref() != Some(id))
        || request.native_session_file.as_deref().is_some_and(|path| omp_path != Some(path))
    {
        return Err(InspectionError::new("herdr_outcome_unknown", "agent.prompt did not acknowledge the exact pinned occupant"));
    }
    Ok(())
}

#[async_trait]
impl OrchestrationHerdr for HerdrCliAdapter {
    async fn runtime(&self, session_id: &str) -> Result<RuntimeView, InspectionError> {
        let (result, endpoint_identity) = self
            .socket_request_with_identity(session_id, "session.snapshot", json!({}), None)
            .await?;
        let parsed = parse_snapshot(json!({"result": &result}), session_id, &endpoint_identity)?;
        let raw = result
            .get("snapshot")
            .ok_or_else(|| InspectionError::new("malformed_json", "snapshot is required"))?;
        let raw_panes = raw.get("panes").and_then(Value::as_array);
        let raw_agents = raw.get("agents").and_then(Value::as_array);
        let raw_workspaces = raw.get("workspaces").and_then(Value::as_array);
        let workspaces = parsed
            .spaces
            .iter()
            .map(|space| {
                let cwd = raw_workspaces
                    .and_then(|items| {
                        items.iter().find(|item| {
                            item.get("workspace_id").and_then(Value::as_str) == Some(&space.id)
                        })
                    })
                    .and_then(|item| item.get("cwd"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| space.git.as_ref().map(|git| git.checkout_path.clone()))
                    .or_else(|| {
                        parsed
                            .panes
                            .iter()
                            .find(|pane| pane.space_id == space.id)
                            .and_then(|pane| pane.cwd.clone())
                    })
                    .unwrap_or_default();
                RuntimeWorkspace {
                    workspace_id: space.id.clone(),
                    label: space.label.clone(),
                    cwd,
                }
            })
            .collect();
        let panes = parsed
            .panes
            .iter()
            .map(|pane| {
                let raw_pane = raw_panes.and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("pane_id").and_then(Value::as_str) == Some(&pane.id))
                });
                let raw_agent = raw_agents.and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("pane_id").and_then(Value::as_str) == Some(&pane.id))
                });
                let agent = raw_agent.or(raw_pane);
                let omp_path = agent.and_then(observed_omp_session_path);
                let native_session_id = agent.and_then(|agent| observed_session_id(agent, omp_path));
                RuntimePane {
                    workspace_id: pane.space_id.clone(),
                    workspace_label: parsed
                        .spaces
                        .iter()
                        .find(|space| space.id == pane.space_id)
                        .map(|space| space.label.clone())
                        .unwrap_or_default(),
                    tab_id: pane.tab_id.clone(),
                    tab_label: parsed
                        .tabs
                        .iter()
                        .find(|tab| tab.id == pane.tab_id)
                        .map(|tab| tab.label.clone())
                        .unwrap_or_default(),
                    pane_id: pane.id.clone(),
                    terminal_id: Some(pane.terminal_id.clone()),
                    native_session_id,
                    native_session_file: omp_path.map(str::to_owned),
                    agent_name: agent
                        .and_then(|item| item.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    agent_status: Some(pane.agent_status.clone()),
                    agent_kind: agent
                        .and_then(|item| item.get("agent"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    launch_pending: agent
                        .and_then(|item| item.get("launch_pending"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    state_changed_at: agent
                        .and_then(|item| item.get("state_changed_at"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                }
            })
            .collect();
        Ok(RuntimeView {
            endpoint_identity,
            boot_id: raw
                .get("boot_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            workspaces,
            panes,
        })
    }

    async fn create_agent_tab(
        &self,
        session_id: &str,
        request: &AgentTabRequest,
    ) -> Result<RunLocation, InspectionError> {
        validate_agent_tab_request(request)?;
        let env = launch_environment(&request.env)?;
        let (result, _) = self.socket_request_with_identity(session_id, "tab.create", json!({"workspace_id":request.workspace_id,"cwd":request.cwd,"label":request.label,"focus":false,"env":env}), Some(&request.endpoint_identity)).await.map_err(unknown)?;
        let tab_id = super::projects::parse_tab_result(&result, &request.workspace_id)
            .map_err(|error| InspectionError::new("herdr_outcome_unknown", error.message))?;
        let runtime = self
            .runtime(session_id)
            .await
            .map_err(|error| InspectionError::new("herdr_outcome_unknown", error.message))?;
        if runtime.endpoint_identity != request.endpoint_identity {
            return Err(InspectionError::new(
                "herdr_outcome_unknown",
                "Endpoint changed after tab creation",
            ));
        }
        let panes: Vec<_> = runtime
            .panes
            .iter()
            .filter(|pane| pane.workspace_id == request.workspace_id && pane.tab_id == tab_id)
            .collect();
        if panes.len() != 1 {
            return Err(InspectionError::new(
                "herdr_outcome_unknown",
                "Created tab must contain exactly one authoritative pane",
            ));
        }
        let pane = panes[0];
        Ok(RunLocation {
            endpoint_identity: runtime.endpoint_identity,
            session_id: session_id.into(),
            workspace_id: request.workspace_id.clone(),
            tab_id,
            pane_id: pane.pane_id.clone(),
            launch_tag: request.launch_tag.clone(),
            boot_id: runtime.boot_id,
            terminal_id: pane.terminal_id.clone(),
            native_session_id: pane.native_session_id.clone(),
        })
    }

    async fn create_direct_agent_tab(
        &self,
        session_id: &str,
        request: &DirectAgentTabRequest,
    ) -> Result<RunLocation, InspectionError> {
        let params = direct_agent_tab_params(request)?;
        let before = self.runtime(session_id).await?;
        if before.endpoint_identity != request.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Endpoint changed before direct tab creation; mutation was not dispatched",
            ));
        }
        if before.panes.iter().any(|pane| pane.tab_label == request.launch_tag) {
            return Err(InspectionError::new(
                "herdr_outcome_unknown",
                "Launch tag already exists; reconcile the recorded launch intent instead of replaying it",
            ));
        }
        // One supported effect, never agent.start, terminal typing or a retry.
        let (result, _) = self.socket_request_with_identity(
            session_id, "layout.apply", params, Some(&request.endpoint_identity),
        ).await.map_err(unknown)?;
        let (tab_id, pane_id) = direct_launch_receipt(&result, request)?;
        let runtime = self.runtime(session_id).await.map_err(|error| {
            InspectionError::new("herdr_outcome_unknown", error.message)
        })?;
        direct_launch_location(session_id, request, tab_id, pane_id, &before, &runtime)
    }

    async fn prompt_agent(
        &self,
        session_id: &str,
        request: &AgentPromptRequest,
    ) -> Result<(), InspectionError> {
        if request.location.session_id != session_id {
            return Err(InspectionError::new("stale_identity", "Prompt request belongs to a different physical session"));
        }
        validate_prompt_target(&self.runtime(session_id).await?, request)?;
        // Supported text+Enter submission only: never wait, retry, or fall back to keys.
        let (result, _) = self.socket_request_with_identity(
            session_id, "agent.prompt",
            json!({"target": request.location.pane_id, "text": request.text}),
            Some(&request.location.endpoint_identity),
        ).await.map_err(unknown)?;
        validate_prompt_receipt(&result, request)
    }

    async fn pane_process_info(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        pane_id: &str,
    ) -> Result<PaneProcessInfo, InspectionError> {
        let (result, _) = self.socket_request_with_identity(
            session_id,
            "pane.process_info",
            json!({"pane_id": pane_id}),
            Some(endpoint_identity),
        ).await?;
        let mut info = parse_pane_process_info(&result, pane_id)?;
        info.shell_identity = info.shell_pid
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(|pid| cockpit_core::process_identity::start_identity(pid).map(|start| (pid, start)))
            .and_then(|(pid, start)| cockpit_core::process_identity::executable_identity(pid, start).ok());
        Ok(info)
    }


    async fn close_pane(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        pane_id: &str,
    ) -> Result<(), InspectionError> {
        let (result, _) = self.socket_request_with_identity(
            session_id,
            "pane.close",
            json!({"pane_id": pane_id}),
            Some(endpoint_identity),
        ).await.map_err(unknown)?;
        validate_close_receipt(&result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    struct OmpJournal {
        root: std::path::PathBuf,
        path: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl OmpJournal {
        fn new(id: &str) -> Self {
            let journal = Self::without_header();
            journal.write_id(id);
            journal
        }

        fn without_header() -> Self {
            use std::sync::atomic::Ordering;
            use super::super::NEXT_REQUEST_ID;

            let root = std::env::temp_dir().join(format!(
                "cockpit-herdr-path-{}-{}",
                std::process::id(), NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&root).unwrap();
            Self {
                path: root.join("filename-is-not-the-header-id.jsonl"),
                root,
            }
        }

        fn write_id(&self, id: &str) {
            std::fs::write(&self.path, format!("{}\n", json!({"type": "session", "id": id}))).unwrap();
        }
    }

    #[cfg(unix)]
    impl Drop for OmpJournal {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[cfg(unix)]
    fn omp_path_session(path: &Path) -> Value {
        json!({"source": "herdr:omp", "agent": "omp", "kind": "path", "value": path})
    }

    #[cfg(unix)]
    fn omp_path_snapshot(path: &Path) -> Value {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/session-snapshot.json"
        )).unwrap();
        let mut result = fixture["result"].clone();
        let snapshot = &mut result["snapshot"];
        snapshot["panes"][0]["agent"] = json!("omp");
        snapshot["panes"][0]["agent_status"] = json!("idle");
        snapshot["panes"][0]["agent_session"] = omp_path_session(path);
        snapshot["agents"][0]["agent"] = json!("omp");
        snapshot["agents"][0]["agent_status"] = json!("idle");
        snapshot["agents"][0]["terminal_id"] = snapshot["panes"][0]["terminal_id"].clone();
        snapshot["agents"][0]["launch_pending"] = json!(false);
        snapshot["agents"][0]["agent_session"] = omp_path_session(path);
        result
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_projects_only_supported_path_headers_and_preserves_generic_ids() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;
        use super::super::HerdrCliConfig;

        for raw_agent_present in [true, false] {
            for case in 0..16 {
                let journal = OmpJournal::new("actual opaque session: one");
                let socket = journal.root.join("runtime.sock");
                let listener = UnixListener::bind(&socket).unwrap();
                let mut result = omp_path_snapshot(&journal.path);
                let snapshot = &mut result["snapshot"];
                let agent = if raw_agent_present {
                    // A matching raw agent still takes precedence over the raw pane,
                    // even when the agent's optional path observation is unsupported.
                    snapshot["panes"][0]["agent_session"] = json!({"kind": "id", "value": "ignored-pane-id"});
                    &mut snapshot["agents"][0]
                } else {
                    snapshot["agents"] = json!([]);
                    &mut snapshot["panes"][0]
                };
                match case {
                    1 => agent["agent_session"]["source"] = json!("other:omp"),
                    2 => agent["agent"] = json!("generic-harness"),
                    3 => agent["agent_session"]["agent"] = json!("generic-harness"),
                    4 => { agent["agent_session"].as_object_mut().unwrap().remove("source"); }
                    5 => agent["agent_session"]["value"] = json!("relative/session.jsonl"),
                    6 => agent["agent_session"]["kind"] = json!("unknown"),
                    7 => { std::fs::remove_file(&journal.path).unwrap(); }
                    8 => agent["agent_session"]["value"] = json!(journal.root),
                    9 => { std::fs::write(&journal.path, "not a session header\n").unwrap(); }
                    10 => { std::fs::write(&journal.path, "x".repeat(64 * 1024 + 1)).unwrap(); }
                    11 => {
                        agent["agent"] = json!("generic-harness");
                        agent["agent_session"] = json!({"kind": "id", "value": " raw generic: opaque -id "});
                    }
                    12 => { agent["agent_session"].as_object_mut().unwrap().remove("kind"); }
                    13 => agent["agent_session"]["value"] = json!("/tmp/control\nsession.jsonl"),
                    14 => agent["agent_session"]["value"] = json!(null),
                    15 => agent["agent_session"]["value"] = json!(""),
                    _ => {}
                }
                let expected = match case {
                    0 => Some("actual opaque session: one"),
                    11 => Some(" raw generic: opaque -id "),
                    _ => None,
                };
                let server = tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let request: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request["method"], "session.snapshot");
                    reader.into_inner().write_all(format!("{}\n",
                        json!({"id": request["id"], "result": result})).as_bytes()).await.unwrap();
                });
                let config = HerdrCliConfig::from_options(
                    None, Some("fixture".into()), Some(socket),
                ).unwrap();
                let runtime = HerdrCliAdapter::new(config).runtime("fixture").await.unwrap();
                assert_eq!(runtime.panes.len(), 1);
                assert_eq!(runtime.panes[0].native_session_id.as_deref(), expected,
                    "raw agent {raw_agent_present}, case {case}");
                let expected_path = match case {
                    0 | 7 | 9 | 10 => Some(journal.path.as_path()),
                    8 => Some(journal.root.as_path()),
                    _ => None,
                };
                assert_eq!(runtime.panes[0].native_session_file.as_deref(),
                    expected_path.and_then(Path::to_str),
                    "raw agent {raw_agent_present}, case {case}");
                server.await.unwrap();
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_preserves_advertised_path_before_creation_then_reads_actual_header() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;
        use super::super::HerdrCliConfig;

        let journal = OmpJournal::without_header();
        assert!(!journal.path.exists());
        let socket = journal.root.join("lazy-runtime.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let snapshot = omp_path_snapshot(&journal.path);
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["method"], "session.snapshot");
                reader.into_inner().write_all(format!("{}\n",
                    json!({"id": request["id"], "result": snapshot})).as_bytes()).await.unwrap();
            }
        });
        let config = HerdrCliConfig::from_options(
            None, Some("fixture".into()), Some(socket),
        ).unwrap();
        let adapter = HerdrCliAdapter::new(config);
        let initial = adapter.runtime("fixture").await.unwrap();
        assert_eq!(initial.panes[0].native_session_file.as_deref(), journal.path.to_str());
        assert_eq!(initial.panes[0].native_session_id, None);

        // The first skill/tool turn creates the advertised file; its basename
        // is deliberately unrelated to the actual opaque session header ID.
        journal.write_id("actual lazy header: not the filename");
        let materialized = adapter.runtime("fixture").await.unwrap();
        assert_eq!(materialized.panes[0].native_session_file.as_deref(), journal.path.to_str());
        assert_eq!(materialized.panes[0].native_session_id.as_deref(),
            Some("actual lazy header: not the filename"));
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn path_prompt_rechecks_fresh_headers_and_preserves_unknown_receipts_without_retry() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::{UnixListener, UnixStream};
        use super::super::HerdrCliConfig;

        for case in 0..16 {
            let journal = if case == 15 {
                OmpJournal::without_header()
            } else {
                OmpJournal::new("actual opaque session: one")
            };
            let replacement = OmpJournal::new(if case >= 10 {
                "actual opaque session: one"
            } else {
                "replacement opaque session: two"
            });
            let socket = journal.root.join("prompt.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let probe = UnixStream::connect(&socket).await.unwrap();
            let endpoint = HerdrCliAdapter::socket_peer_identity(&socket, &probe).unwrap();
            drop(probe);
            drop(listener.accept().await.unwrap());
            let original = omp_path_snapshot(&journal.path);
            let mut fresh = original.clone();
            if matches!(case, 2 | 10 | 11) {
                fresh["snapshot"]["agents"][0]["agent_session"] = omp_path_session(&replacement.path);
            }
            if case == 13 {
                fresh["snapshot"]["agents"][0].as_object_mut().unwrap().remove("agent_session");
            }
            let journal_path = journal.path.clone();
            let replacement_path = replacement.path.clone();
            let server = tokio::spawn(async move {
                // Initial observation pins the real header, then prompt_agent must
                // fetch a second snapshot rather than trust that earlier projection.
                for result in [original, fresh.clone()] {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let actual: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(actual["method"], "session.snapshot");
                    reader.into_inner().write_all(format!("{}\n",
                        json!({"id": actual["id"], "result": result})).as_bytes()).await.unwrap();
                }
                let mut prompt_calls = 0;
                if !matches!(case, 1..=4 | 11 | 13) {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let actual: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(actual["method"], "agent.prompt");
                    assert_eq!(actual["params"], json!({
                        "target": "pane-a", "text": "Cockpit inbox: fetch with cockpit-cli inbox list.",
                    }));
                    prompt_calls += 1;
                    // The receipt comes from observed state, not from the request's
                    // pinned ID. The header can change after an accepted submission.
                    let mut receipt_agent = fresh["snapshot"]["agents"][0].clone();
                    match case {
                        5 => { std::fs::write(&journal_path,
                            format!("{}\n", json!({"type": "session", "id": "replacement opaque session: two"}))).unwrap(); }
                        6 => receipt_agent["agent_session"] = omp_path_session(&replacement_path),
                        7 => { std::fs::remove_file(&journal_path).unwrap(); }
                        8 => { std::fs::write(&journal_path, "not a session header\n").unwrap(); }
                        12 => receipt_agent["agent_session"] = omp_path_session(&replacement_path),
                        14 => { receipt_agent.as_object_mut().unwrap().remove("agent_session"); }
                        _ => {}
                    }
                    reader.into_inner().write_all(format!("{}\n", json!({
                        "id": actual["id"], "result": {"type": "agent_prompted", "agent": receipt_agent},
                    })).as_bytes()).await.unwrap();
                }
                assert!(tokio::time::timeout(
                    std::time::Duration::from_millis(30), listener.accept(),
                ).await.is_err(), "No refused submission or uncertain receipt may be replayed");
                prompt_calls
            });
            let config = HerdrCliConfig::from_options(
                None, Some("fixture".into()), Some(socket),
            ).unwrap();
            let adapter = HerdrCliAdapter::new(config);
            let runtime = adapter.runtime("fixture").await.unwrap();
            let pane = &runtime.panes[0];
            assert_eq!(pane.native_session_id.as_deref(),
                if case == 15 { None } else { Some("actual opaque session: one") });
            assert_eq!(pane.native_session_file.as_deref(), journal.path.to_str());
            let request = AgentPromptRequest {
                location: RunLocation {
                    endpoint_identity: endpoint, session_id: "fixture".into(),
                    workspace_id: pane.workspace_id.clone(), tab_id: pane.tab_id.clone(),
                    pane_id: pane.pane_id.clone(), launch_tag: pane.tab_label.clone(),
                    boot_id: runtime.boot_id.clone(), terminal_id: pane.terminal_id.clone(),
                    native_session_id: if case == 9 || case >= 11 { None } else { pane.native_session_id.clone() },
                },
                harness_kind: "omp".into(),
                native_session_file: (case >= 11).then(|| pane.native_session_file.clone().unwrap()),
                text: "Cockpit inbox: fetch with cockpit-cli inbox list.".into(),
            };
            match case {
                1 => journal.write_id("replacement opaque session: two"),
                3 | 9 => { std::fs::remove_file(&journal.path).unwrap(); }
                4 => { std::fs::write(&journal.path, "not a session header\n").unwrap(); }
                15 => journal.write_id("actual opaque session: one"),
                _ => {}
            }
            let result = adapter.prompt_agent("fixture", &request).await;
            if matches!(case, 0 | 9 | 10 | 15) {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().code, if matches!(case, 1..=4 | 11 | 13) {
                    "stale_identity"
                } else {
                    "herdr_outcome_unknown"
                }, "case {case}");
            }
            assert_eq!(server.await.unwrap(), if matches!(case, 1..=4 | 11 | 13) { 0 } else { 1 },
                "case {case}");
        }
    }


    fn direct_launch_request() -> DirectAgentTabRequest {
        DirectAgentTabRequest {
            endpoint_identity: "endpoint".into(),
            workspace_id: "workspace".into(),
            cwd: "/work/repository".into(),
            label: "launch-tag".into(),
            env: BTreeMap::from([
                ("COCKPIT_LAUNCH_TAG".into(), "launch-tag".into()),
                ("COCKPIT_JOB_CREDENTIAL_FILE".into(), "/private/credential".into()),
                ("COCKPIT_ATTACHMENT_ID".into(), "attachment".into()),
                ("COCKPIT_ATTACHMENT_EPOCH".into(), "2".into()),
            ]),
            launch_tag: "launch-tag".into(),
            command: vec!["/usr/bin/python3".into(), "agent.py".into(), "literal ; $(text)".into()],
        }
    }

    #[test]
    fn direct_launch_uses_stock_new_unfocused_layout_with_literal_argv() {
        let request = direct_launch_request();
        let params = direct_agent_tab_params(&request).unwrap();
        assert_eq!(params, json!({
            "workspace_id": "workspace",
            "tab_label": "launch-tag",
            "focus": false,
            "root": {
                "type": "pane",
                "label": "launch-tag",
                "cwd": "/work/repository",
                "command": request.command,
                "env": request.env,
            },
        }));
        assert!(params.get("tab_id").is_none());
        assert!(params["root"].get("pane_id").is_none());
        assert!(params["root"].get("respawn_shell_on_exit").is_none());
    }

    #[test]
    fn direct_launch_shares_request_and_environment_validation_without_shell_fallback() {
        for change in 0..7 {
            let mut request = direct_launch_request();
            match change {
                0 => request.cwd = "relative".into(),
                1 => request.label = "different-tag".into(),
                2 => request.launch_tag.clear(),
                3 => request.command.clear(),
                4 => request.command[0] = "-not-an-executable".into(),
                5 => request.command.push("invalid\0argument".into()),
                _ => { request.env.insert("UNSUPPORTED".into(), "value".into()); }
            }
            assert!(direct_agent_tab_params(&request).is_err(), "change {change}");
        }
    }

    fn direct_receipt(request: &DirectAgentTabRequest) -> Value {
        json!({
            "type": "layout_apply",
            "layout": {
                "workspace_id": request.workspace_id,
                "tab_id": "new-tab",
                "zoomed": false,
                "focused_pane_id": "new-pane",
                "root": {
                    "type": "pane",
                    "pane_id": "new-pane",
                    "label": request.launch_tag,
                    "cwd": request.cwd,
                    "command": request.command,
                },
            },
        })
    }

    #[test]
    fn direct_launch_refuses_malformed_or_contradictory_stock_receipts() {
        let request = direct_launch_request();
        let receipt = direct_receipt(&request);
        assert_eq!(direct_launch_receipt(&receipt, &request).unwrap(), ("new-tab", "new-pane"));
        for change in 0..9 {
            let mut receipt = receipt.clone();
            match change {
                0 => receipt["type"] = json!("tab_created"),
                1 => receipt["layout"]["workspace_id"] = json!("other-workspace"),
                2 => receipt["layout"]["tab_id"] = json!(""),
                3 => receipt["layout"]["root"]["pane_id"] = Value::Null,
                4 => receipt["layout"]["root"]["type"] = json!("split"),
                5 => receipt["layout"]["root"]["label"] = json!("other-launch"),
                6 => receipt["layout"]["focused_pane_id"] = json!("other-pane"),
                7 => receipt["layout"]["root"]["command"] = json!(["sh", "-c", "exec omp"]),
                _ => receipt["layout"]["root"]["command"] = Value::Null,
            }
            assert_eq!(direct_launch_receipt(&receipt, &request).unwrap_err().code, "herdr_outcome_unknown");
        }
    }

    #[test]
    fn direct_launch_location_requires_fresh_unique_pinned_runtime_not_agent_ack() {
        let request = direct_launch_request();
        let (mut runtime, _) = owned_launch_fixture();
        let before = runtime.clone();
        runtime.workspaces.push(RuntimeWorkspace {
            workspace_id: request.workspace_id.clone(),
            label: "workspace".into(),
            cwd: request.cwd.clone(),
        });
        let mut pane = runtime.panes[0].clone();
        pane.tab_id = "new-tab".into();
        pane.pane_id = "new-pane".into();
        pane.tab_label = request.launch_tag.clone();
        pane.terminal_id = Some("new-terminal".into());
        // No agent.start metadata or extension registration is required for a
        // location receipt, and neither is fabricated from the launch ACK.
        pane.native_session_id = None;
        pane.agent_kind = None;
        runtime.panes.push(pane);
        let location = direct_launch_location("fixture", &request, "new-tab", "new-pane", &before, &runtime).unwrap();
        assert_eq!(location.terminal_id.as_deref(), Some("new-terminal"));
        assert_eq!(location.native_session_id, None);
        for change in 0..13 {
            let mut before = before.clone();
            let mut runtime = runtime.clone();
            match change {
                0 => runtime.endpoint_identity = "other-endpoint".into(),
                1 => runtime.boot_id = Some("other-boot".into()),
                2 => runtime.panes[2].workspace_id = "other-workspace".into(),
                3 => runtime.panes[2].pane_id = "other-pane".into(),
                4 => runtime.panes[2].tab_label = "other-launch".into(),
                5 => runtime.panes[2].terminal_id = None,
                6 => runtime.panes[2].terminal_id = Some(String::new()),
                7 => runtime.panes[0].terminal_id = Some("new-terminal".into()),
                8 => runtime.panes[0].tab_id = "new-tab".into(),
                9 => runtime.panes[0].tab_label = request.launch_tag.clone(),
                10 => before.panes.push(runtime.panes[2].clone()),
                11 => runtime.workspaces.clear(),
                _ => runtime.panes[0].pane_id = "new-pane".into(),
            }
            assert_eq!(
                direct_launch_location("fixture", &request, "new-tab", "new-pane", &before, &runtime).unwrap_err().code,
                "herdr_outcome_unknown", "change {change}",
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_launch_dispatches_one_stock_effect_and_pins_fresh_receipt() {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::{UnixListener, UnixStream};
        use super::super::{HerdrCliConfig, NEXT_REQUEST_ID};

        for outcome in 0..4 {
            let socket = std::env::temp_dir().join(format!(
                "cockpit-herdr-direct-{}-{}.sock",
                std::process::id(), NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
            ));
            let listener = UnixListener::bind(&socket).unwrap();
            let probe = UnixStream::connect(&socket).await.unwrap();
            let endpoint = HerdrCliAdapter::socket_peer_identity(&socket, &probe).unwrap();
            drop(probe);
            drop(listener.accept().await.unwrap());
            let mut request = direct_launch_request();
            request.endpoint_identity = endpoint.clone();
            request.workspace_id = "space-a".into();
            let fixture: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/session-snapshot.json"
            )).unwrap();
            let before = fixture["result"].clone();
            let mut after = before.clone();
            let snapshot = &mut after["snapshot"];
            let mut tab = snapshot["tabs"][0].clone();
            tab["tab_id"] = json!("new-tab");
            tab["label"] = json!(request.launch_tag);
            snapshot["tabs"].as_array_mut().unwrap().push(tab);
            let mut pane = snapshot["panes"][0].clone();
            pane["tab_id"] = json!("new-tab");
            pane["pane_id"] = json!("new-pane");
            pane["terminal_id"] = json!("new-terminal");
            pane["agent"] = Value::Null;
            snapshot["panes"].as_array_mut().unwrap().push(pane);
            let mut receipt = direct_receipt(&request);
            if outcome == 1 {
                receipt["layout"]["workspace_id"] = json!("foreign-workspace");
            }
            let mut exchanges = vec![
                ("session.snapshot", json!({}), json!({"result": before})),
                ("layout.apply", direct_agent_tab_params(&request).unwrap(), if outcome >= 2 {
                    json!({"error":{
                        "code": if outcome == 2 { "request_outcome_unknown" } else { "layout_apply_failed" },
                        "message":"effect uncertain",
                    }})
                } else {
                    json!({"result": receipt})
                }),
            ];
            if outcome == 0 {
                exchanges.push(("session.snapshot", json!({}), json!({"result": after})));
            }
            let server = tokio::spawn(async move {
                for (method, params, mut response) in exchanges {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let actual: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(actual["method"], method);
                    assert_eq!(actual["params"], params);
                    response["id"] = actual["id"].clone();
                    reader.into_inner().write_all(format!("{response}\n").as_bytes()).await.unwrap();
                }
            });
            let config = HerdrCliConfig::from_options(
                None, Some("fixture".into()), Some(socket.clone()),
            ).unwrap();
            let result = HerdrCliAdapter::new(config).create_direct_agent_tab("fixture", &request).await;
            if outcome == 0 {
                let location = result.unwrap();
                assert_eq!(location.endpoint_identity, endpoint);
                assert_eq!(location.workspace_id, "space-a");
                assert_eq!(location.tab_id, "new-tab");
                assert_eq!(location.pane_id, "new-pane");
                assert_eq!(location.terminal_id.as_deref(), Some("new-terminal"));
                assert_eq!(location.launch_tag, request.launch_tag);
                assert_eq!(location.native_session_id, None);
            } else {
                assert_eq!(result.unwrap_err().code, "herdr_outcome_unknown");
            }
            server.await.unwrap();
            std::fs::remove_file(socket).unwrap();
        }
    }


    fn owned_launch_fixture() -> (RuntimeView, RunLocation) {
        let location = RunLocation {
            endpoint_identity: "endpoint".into(),
            session_id: "fixture".into(),
            workspace_id: "workspace".into(),
            tab_id: "owned-tab".into(),
            pane_id: "owned-pane".into(),
            launch_tag: "original-tag".into(),
            boot_id: Some("boot".into()),
            terminal_id: Some("owned-terminal".into()),
            native_session_id: None,
        };
        let pane = RuntimePane {
            workspace_id: location.workspace_id.clone(),
            workspace_label: "User renamed workspace".into(),
            tab_id: location.tab_id.clone(),
            tab_label: "User renamed tab".into(),
            pane_id: location.pane_id.clone(),
            terminal_id: location.terminal_id.clone(),
            native_session_id: None,
            native_session_file: None,
            agent_name: None,
            agent_kind: None,
            launch_pending: false,
            agent_status: None,
            state_changed_at: None,
        };
        let mut sibling = pane.clone();
        sibling.tab_id = "user-tab".into();
        sibling.pane_id = "user-pane".into();
        sibling.terminal_id = Some("user-terminal".into());
        (RuntimeView {
            endpoint_identity: location.endpoint_identity.clone(),
            boot_id: location.boot_id.clone(),
            workspaces: vec![],
            panes: vec![pane, sibling],
        }, location)
    }

    #[test]
    fn preserved_working_terminal_has_no_agent_launch_environment_or_tag() {
        let ordinary = AgentTabRequest {
            endpoint_identity: "endpoint".into(),
            workspace_id: "workspace".into(),
            cwd: "/work/repository".into(),
            label: "Working terminal".into(),
            env: BTreeMap::new(),
            launch_tag: String::new(),
        };
        validate_agent_tab_request(&ordinary).unwrap();
        for change in 0..3 {
            let mut request = ordinary.clone();
            match change {
                0 => request.cwd = "relative".into(),
                1 => request.label = "Arbitrary tab".into(),
                _ => { request.env.insert("COCKPIT_RUN_ID".into(), "run".into()); }
            }
            assert_eq!(validate_agent_tab_request(&request).unwrap_err().code, "invalid_launch_request");
        }
        let mut launch = ordinary;
        launch.launch_tag = "launch-tag".into();
        launch.label = launch.launch_tag.clone();
        launch.env.insert("COCKPIT_RUN_ID".into(), "run".into());
        validate_agent_tab_request(&launch).unwrap();
    }


    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_preserves_optional_display_name_without_falling_back_to_kind() {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        use super::super::{HerdrCliConfig, NEXT_REQUEST_ID};

        for raw_agent_present in [true, false] {
            for name in [Some(Value::Null), None, Some(json!("renamed worker"))] {
                let socket = std::env::temp_dir().join(format!(
                    "cockpit-herdr-runtime-alias-{}-{}.sock",
                    std::process::id(),
                    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
                ));
                let listener = UnixListener::bind(&socket).unwrap();
                let mut fixture: Value = serde_json::from_str(include_str!(
                    "../../tests/fixtures/session-snapshot.json"
                ))
                .unwrap();
                let snapshot = &mut fixture["result"]["snapshot"];
                snapshot["panes"][0]["agent"] = json!("omp");
                snapshot["panes"][0]["name"] = json!("stale pane alias");
                let agent = if raw_agent_present {
                    &mut snapshot["agents"][0]
                } else {
                    snapshot["agents"] = json!([]);
                    &mut snapshot["panes"][0]
                };
                agent["agent"] = json!("omp");
                agent["agent_session"] = json!({"kind": "id", "value": "native-session"});
                agent["launch_pending"] = json!(false);
                agent["state_changed_at"] = json!("2026-10-07T12:00:00Z");
                let expected_name = name.as_ref().and_then(Value::as_str).map(str::to_owned);
                if let Some(name) = name {
                    agent["name"] = name;
                } else {
                    agent.as_object_mut().unwrap().remove("name");
                }
                let ui_snapshot =
                    parse_snapshot(fixture.clone(), "default", "fixture-endpoint").unwrap();
                assert_eq!(ui_snapshot.panes[0].agent.as_deref(), Some("omp"));
                if raw_agent_present {
                    assert_eq!(ui_snapshot.agents[0].name, "omp");
                }
                let result = fixture["result"].clone();
                let server = tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let request: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request["method"], "session.snapshot");
                    reader
                        .into_inner()
                        .write_all(
                            format!("{}\n", json!({"id": request["id"], "result": result}))
                                .as_bytes(),
                        )
                        .await
                        .unwrap();
                });
                let config = HerdrCliConfig::from_options(
                    None,
                    Some("default".into()),
                    Some(socket.clone()),
                )
                .unwrap();
                let runtime = HerdrCliAdapter::new(config).runtime("default").await.unwrap();
                assert_eq!(runtime.panes.len(), 1);
                let pane = &runtime.panes[0];
                assert_eq!(pane.agent_name, expected_name);
                assert_eq!(pane.agent_kind.as_deref(), Some("omp"));
                assert_eq!(pane.native_session_id.as_deref(), Some("native-session"));
                assert_eq!(pane.agent_status.as_deref(), Some("working"));
                assert!(!pane.launch_pending);
                assert_eq!(
                    pane.state_changed_at.as_deref(),
                    Some("2026-10-07T12:00:00Z")
                );
                server.await.unwrap();
                std::fs::remove_file(socket).unwrap();
            }
        }
    }

    #[test]
    fn orchestration_launch_environment_is_harness_neutral_and_never_invents_conversation() {
        let env = BTreeMap::from([
            ("COCKPIT_HARNESS".into(), "probe".into()),
            ("COCKPIT_HERDR_SESSION".into(), "physical-session".into()),
            ("COCKPIT_JOB_CREDENTIAL_FILE".into(), "/private/credential".into()),
        ]);
        assert_eq!(launch_environment(&env).unwrap(), json!(env));
        for key in ["COCKPIT_SESSION_ID", "PI_CODING_AGENT_DIR", "PI_CODING_AGENT_SESSION_DIR", "COCKPIT_OMP_EXTENSION"] {
            let mut env = env.clone();
            env.insert(key.into(), "invented-or-obsolete".into());
            assert_eq!(launch_environment(&env).unwrap_err().code, "invalid_environment");
        }
    }

    #[test]
    fn launch_environment_rejects_shell_and_unbounded_keys() {
        assert!(launch_environment(&BTreeMap::from([("SHELL".into(), "/bin/sh".into())])).is_err());
        assert!(
            launch_environment(&BTreeMap::from([(
                "COCKPIT_CLI_PATH".into(),
                "/bin/cockpit-cli\nexec bad".into()
            )]))
            .is_err()
        );
        for (length, valid) in [(4096, true), (4097, false)] {
            let env = BTreeMap::from([("COCKPIT_CLI_PATH".into(), "x".repeat(length))]);
            assert_eq!(launch_environment(&env).is_ok(), valid, "length {length}");
        }
    }

    #[test]
    fn process_info_parses_pid_group_and_names() {
        let response = json!({
            "type": "pane_process_info",
            "process_info": {
                "pane_id": "w1:p2",
                "shell_pid": 12,
                "foreground_process_group_id": 20,
                "foreground_processes": [
                    {"pid": 20, "name": "omp", "argv": ["omp"], "cwd": "/tmp"},
                    {"pid": 21, "name": "child"}
                ],
                "tty": "/dev/pts/1"
            }
        });
        let parsed = parse_pane_process_info(&response, "w1:p2").unwrap();
        assert_eq!(parsed.pane_id, "w1:p2");
        assert_eq!(parsed.shell_pid, Some(12));
        assert_eq!(parsed.foreground_pgid, Some(20));
        assert_eq!(parsed.processes, vec![(20, "omp".into()), (21, "child".into())]);
    }

    #[test]
    fn process_info_does_not_trust_wire_supplied_shell_fingerprint() {
        let response = json!({"type":"pane_process_info","process_info":{
            "pane_id":"w1:p2","shell_pid":12,
            "shell_identity":{
                "process":{"pid":12,"start_ticks":123,"kernel_boot_id":"boot"},
                "executable_device":"1","executable_inode":"2","argv_digest":"forged"
            }
        }});
        assert!(parse_pane_process_info(&response, "w1:p2").unwrap().shell_identity.is_none());
    }

    #[test]
    fn process_info_missing_optional_evidence_remains_unobserved() {
        let response = json!({"type":"pane_process_info","process_info":{"pane_id":"w1:p2"}});
        let parsed = parse_pane_process_info(&response, "w1:p2").unwrap();
        assert_eq!(parsed.shell_pid, None);
        assert_eq!(parsed.foreground_pgid, None);
        assert!(parsed.processes.is_empty());
        let nulls = json!({"type":"pane_process_info","process_info":{
            "pane_id":"w1:p2","shell_pid":null,"foreground_process_group_id":null
        }});
        assert_eq!(parse_pane_process_info(&nulls, "w1:p2").unwrap(), parsed);
    }

    #[test]
    fn process_info_rejects_wrong_pane_type_and_malformed_pids() {
        let valid = json!({"type":"pane_process_info","process_info":{
            "pane_id":"w1:p2","shell_pid":12,"foreground_process_group_id":12,
            "foreground_processes":[{"pid":12,"name":"sh"}]
        }});
        assert!(parse_pane_process_info(&valid, "w1:p3").is_err());
        for response in [json!(null), json!({}), json!({"type":"ok"}),
            json!({"type":"pane_process_info","process_info":null})] {
            assert!(parse_pane_process_info(&response, "w1:p2").is_err());
        }
        for field in ["shell_pid", "foreground_process_group_id"] {
            for value in [json!(-1), json!(4_294_967_296u64), json!("12"), json!(1.5)] {
                let mut response = valid.clone();
                response["process_info"][field] = value;
                assert_eq!(parse_pane_process_info(&response, "w1:p2").unwrap_err().code, "malformed_response");
            }
        }
        for processes in [json!(null), json!({}), json!([{"pid":12}]),
            json!([{"pid":-1,"name":"sh"}]), json!([{"pid":12,"name":false}])] {
            let mut response = valid.clone();
            response["process_info"]["foreground_processes"] = processes;
            assert!(parse_pane_process_info(&response, "w1:p2").is_err());
        }
    }

    #[test]
    fn close_receipt_never_infers_success_from_an_unrelated_response() {
        assert!(validate_close_receipt(&json!({"type":"ok"})).is_ok());
        for response in [json!(null), json!({}), json!({"type":"pane_info"}),
            json!({"type":true}), json!({"type":"pane_closed"})] {
            assert_eq!(validate_close_receipt(&response).unwrap_err().code, "herdr_outcome_unknown");
        }
    }

    #[test]
    fn close_errors_preserve_refusal_and_pre_dispatch_identity_fences() {
        for code in ["pane_not_found", "confirmation_required", "stale_identity", "request_not_dispatched"] {
            let error = InspectionError::new(code, "specific failure");
            assert_eq!(unknown(error.clone()), error);
        }
        for code in ["request_outcome_unknown", "response_timeout", "malformed_json", "malformed_response", "session_mismatch"] {
            let mapped = unknown(InspectionError::new(code, "specific failure"));
            assert_eq!(mapped.code, "herdr_outcome_unknown");
            assert_eq!(mapped.message, "specific failure");
        }
    }

    fn prompt_receipt(request: &AgentPromptRequest) -> Value {
        json!({"type":"agent_prompted","agent":{
            "pane_id":request.location.pane_id, "terminal_id":request.location.terminal_id,
            "workspace_id":request.location.workspace_id, "tab_id":request.location.tab_id,
            "agent":request.harness_kind,
            "agent_session":request.location.native_session_id.as_ref().map(|id| json!({"kind":"id","value":id})),
        }})
    }

    #[test]
    fn orchestration_prompt_requires_exact_ready_target_and_submission_receipt() {
        let (mut runtime, mut location) = owned_launch_fixture();
        location.native_session_id = Some("observed-occupant".into());
        runtime.panes[0].native_session_id = location.native_session_id.clone();
        runtime.panes[0].tab_label = location.launch_tag.clone();
        let request = AgentPromptRequest {
            location, harness_kind: "generic-harness".into(),
            native_session_file: None,
            text: "Cockpit inbox: fetch with cockpit-cli inbox list.".into(),
        };
        runtime.panes[0].agent_kind = Some("generic-harness".into());
        runtime.panes[0].agent_status = Some("idle".into());
        validate_prompt_target(&runtime, &request).unwrap();
        for change in 0..13 {
            let mut runtime = runtime.clone();
            match change {
                0 => runtime.endpoint_identity = "other-endpoint".into(),
                1 => runtime.panes[0].terminal_id = Some("replacement-terminal".into()),
                2 => runtime.panes.push(runtime.panes[0].clone()),
                3 => runtime.panes[0].launch_pending = true,
                4 => runtime.panes[0].agent_status = Some("working".into()),
                5 => runtime.panes[0].agent_status = Some("blocked".into()),
                6 => runtime.panes[0].agent_kind = Some("replacement-harness".into()),
                7 => runtime.panes[0].tab_label = "replacement-launch".into(),
                8 => runtime.panes[0].tab_id = "replacement-tab".into(),
                9 => runtime.panes[0].workspace_id = "replacement-workspace".into(),
                10 => runtime.panes[0].native_session_id = Some("replacement-same-harness-session".into()),
                11 => runtime.panes[0].native_session_id = None,
                _ => runtime.boot_id = Some("replacement-boot".into()),
            }
            assert!(validate_prompt_target(&runtime, &request).is_err(), "change {change}");
        }
        runtime.panes[0].agent_kind = None;
        assert_eq!(validate_prompt_target(&runtime, &request).unwrap_err().code, "herdr_prompt_unsupported");
        let receipt = prompt_receipt(&request);
        validate_prompt_receipt(&receipt, &request).unwrap();
        for field in ["pane_id", "terminal_id", "workspace_id", "tab_id", "agent"] {
            let mut receipt = receipt.clone();
            receipt["agent"][field] = json!("replacement");
            assert_eq!(validate_prompt_receipt(&receipt, &request).unwrap_err().code, "herdr_outcome_unknown");
        }
        let mut replacement = receipt;
        replacement["agent"]["agent_session"]["value"] = json!("replacement-same-harness-session");
        assert_eq!(validate_prompt_receipt(&replacement, &request).unwrap_err().code, "herdr_outcome_unknown");
        let mut no_native = request.clone();
        no_native.location.native_session_id = None;
        runtime.panes[0].native_session_id = None;
        runtime.panes[0].agent_kind = Some(request.harness_kind.clone());
        validate_prompt_target(&runtime, &no_native).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn orchestration_idle_prompt_without_readiness_extension_dispatches_once_without_uncertain_retry() {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::{UnixListener, UnixStream};
        use super::super::{HerdrCliConfig, NEXT_REQUEST_ID};

        for outcome in 0..9 {
            let socket = std::env::temp_dir().join(format!(
                "cockpit-herdr-prompt-{}-{}.sock", std::process::id(),
                NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
            ));
            let listener = UnixListener::bind(&socket).unwrap();
            let probe = UnixStream::connect(&socket).await.unwrap();
            let endpoint = HerdrCliAdapter::socket_peer_identity(&socket, &probe).unwrap();
            drop(probe);
            drop(listener.accept().await.unwrap());
            let mut fixture: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/session-snapshot.json"
            )).unwrap();
            assert!(fixture["result"]["snapshot"]["agents"][0].get("interactive_ready").is_none());
            assert!(fixture["result"]["snapshot"]["panes"][0].get("interactive_ready").is_none());
            let pane_id = fixture["result"]["snapshot"]["panes"][0]["pane_id"].as_str().unwrap().to_owned();
            let terminal_id = fixture["result"]["snapshot"]["panes"][0]["terminal_id"].as_str().unwrap().to_owned();
            fixture["result"]["snapshot"]["panes"][0]["agent_status"] = json!(if outcome == 4 { "working" } else { "idle" });
            fixture["result"]["snapshot"]["agents"][0]["launch_pending"] = json!(false);
            fixture["result"]["snapshot"]["agents"][0]["agent_session"] = json!({"kind":"id","value":"original-occupant"});
            let request = AgentPromptRequest {
                location: RunLocation {
                    endpoint_identity: endpoint, session_id: "fixture".into(),
                    pane_id: pane_id.clone(), terminal_id: Some(terminal_id),
                    workspace_id: fixture["result"]["snapshot"]["panes"][0]["workspace_id"].as_str().unwrap().into(),
                    tab_id: fixture["result"]["snapshot"]["panes"][0]["tab_id"].as_str().unwrap().into(),
                    launch_tag: fixture["result"]["snapshot"]["tabs"][0]["label"].as_str().unwrap().into(),
                    boot_id: None, native_session_id: Some("original-occupant".into()),
                },
                harness_kind: fixture["result"]["snapshot"]["agents"][0]["agent"].as_str().unwrap().into(),
                native_session_file: None,
                text: "Cockpit inbox: fetch with cockpit-cli inbox list.".into(),
            };
            match outcome {
                5 => fixture["result"]["snapshot"]["agents"][0]["agent"] = json!("replacement-harness"),
                6 => fixture["result"]["snapshot"]["agents"][0]["agent_session"]["value"] = json!("replacement-same-harness-session"),
                7 => fixture["result"]["snapshot"]["tabs"][0]["label"] = json!("replacement-launch"),
                8 => fixture["result"]["snapshot"]["agents"][0]["agent_session"] = Value::Null,
                _ => {}
            }
            let server_request = request.clone();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let actual: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(actual["method"], "session.snapshot");
                reader.into_inner().write_all(format!("{}\n",
                    json!({"id":actual["id"],"result":fixture["result"]})).as_bytes()).await.unwrap();
                if outcome < 4 {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let actual: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(actual["method"], "agent.prompt");
                    assert_eq!(actual["params"], json!({"target":server_request.location.pane_id,"text":server_request.text}));
                    if outcome != 3 {
                        let mut response = match outcome {
                            0 => json!({"result":prompt_receipt(&server_request)}),
                            1 => json!({"error":{"code":"request_outcome_unknown","message":"lost receipt"}}),
                            _ => json!({"result":{"type":"ok"}}),
                        };
                        response["id"] = actual["id"].clone();
                        reader.into_inner().write_all(format!("{response}\n").as_bytes()).await.unwrap();
                    }
                }
                assert!(tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept()).await.is_err(),
                    "No blind retry or raw-key fallback is allowed");
            });
            let config = HerdrCliConfig::from_options(None, Some("fixture".into()), Some(socket.clone())).unwrap();
            let result = HerdrCliAdapter::new(config).prompt_agent("fixture", &request).await;
            if outcome == 0 { result.unwrap(); }
            else { assert_eq!(result.unwrap_err().code, if outcome == 4 { "herdr_prompt_deferred" }
                else if outcome >= 5 { "stale_identity" } else { "herdr_outcome_unknown" }); }
            server.await.unwrap();
            std::fs::remove_file(socket).unwrap();
        }
    }
}
