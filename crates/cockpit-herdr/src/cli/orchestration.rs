use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use cockpit_core::{
    InspectionError,
    orchestration::herdr::{
        AgentStartRequest, AgentTabRequest, OrchestrationHerdr, PaneProcessInfo, RuntimePane,
        RuntimeView, RuntimeWorkspace,
    },
};
use cockpit_protocol::orchestration::RunLocation;
use serde_json::{Value, json};

use super::{HerdrCliAdapter, object, parse_snapshot, required_string};

fn unknown(error: InspectionError) -> InspectionError {
    if matches!(
        error.code.as_str(),
        "request_outcome_unknown" | "malformed_json" | "malformed_response" | "response_timeout"
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

fn validate_start_receipt(
    result: &Value,
    request: &AgentStartRequest,
) -> Result<(), InspectionError> {
    let response = object(&result, "agent.start")
        .map_err(|error| InspectionError::new("herdr_outcome_unknown", error.message))?;
    if required_string(response, "type", "agent.start")
        .ok()
        .as_deref()
        != Some("agent_started")
    {
        return Err(InspectionError::new(
            "herdr_outcome_unknown",
            "agent.start returned no accepted command receipt",
        ));
    }
    let agent = response
        .get("agent")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            InspectionError::new(
                "herdr_outcome_unknown",
                "agent.start receipt missing agent identity",
            )
        })?;
    if agent.get("pane_id").and_then(Value::as_str) != Some(&request.pane_id)
        || agent.get("name").and_then(Value::as_str) != Some(&request.name)
        || agent
            .get("agent")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != request.kind)
    {
        return Err(InspectionError::new(
            "herdr_outcome_unknown",
            "agent.start receipt does not match requested agent/pane",
        ));
    }
    Ok(())
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
                | "COCKPIT_SESSION_ID"
                | "COCKPIT_HERDR_SOCKET"
                | "COCKPIT_HERDR_EXECUTABLE"
                | "COCKPIT_CLI_PATH"
                | "COCKPIT_STATE_ROOT"
                | "COCKPIT_CACHE_ROOT"
                | "COCKPIT_WORKTREE_ROOT"
                | "COCKPIT_COMPANION_ROOT"
                | "COCKPIT_REPOSITORY_ROOTS"
                | "PI_CODING_AGENT_DIR"
                | "PI_CODING_AGENT_SESSION_DIR"
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
                let native_session_id = agent
                    .and_then(|item| item.get("agent_session"))
                    .filter(|session| session.get("kind").and_then(Value::as_str) == Some("id"))
                    .and_then(|session| session.get("value"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
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
                    agent_name: raw_agent
                        .and_then(|agent| agent.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| {
                            parsed
                                .agents
                                .iter()
                                .find(|agent| agent.pane_id == pane.id)
                                .map(|agent| agent.name.clone())
                        })
                        .or_else(|| pane.agent.clone()),
                    agent_status: Some(pane.agent_status.clone()),
                    agent_kind: agent
                        .and_then(|item| item.get("agent"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    launch_pending: agent
                        .and_then(|item| item.get("launch_pending"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    interactive_ready: agent
                        .and_then(|item| item.get("interactive_ready"))
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
        if !Path::new(&request.cwd).is_absolute() || request.label != request.launch_tag {
            return Err(InspectionError::new(
                "invalid_launch_request",
                "Launch requires an absolute cwd and exact tag label",
            ));
        }
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

    async fn start_agent(
        &self,
        session_id: &str,
        request: &AgentStartRequest,
    ) -> Result<(), InspectionError> {
        if request.kind != "omp" || !(3001..=300_000).contains(&request.timeout_ms) {
            return Err(InspectionError::new(
                "invalid_launch_request",
                "Only OMP with a bounded start timeout is supported",
            ));
        }
        // v0.9.3 rejects a typed start before submission while a fresh shell
        // initializes. Mirror only its pinned-terminal, process-info branch;
        // never repeat an accepted or ambiguous command.
        let pinned = self.launch_terminal(session_id, request).await?;
        let mut deadline = None;
        loop {
            match self.socket_request_with_identity(session_id, "agent.start", json!({"pane_id":request.pane_id,"name":request.name,"kind":request.kind,"args":request.args,"timeout_ms":request.timeout_ms}), Some(&request.endpoint_identity)).await {
                Ok((result, _)) => {
                    validate_start_receipt(&result, request)?;
                    if pinned.as_deref().zip(result["agent"]["terminal_id"].as_str())
                        .is_some_and(|(expected, actual)| expected != actual)
                    {
                        return Err(InspectionError::new("herdr_outcome_unknown", "Accepted start receipt belongs to a different terminal; no additional start was sent"));
                    }
                    return Ok(());
                }
                Err(error) if error.code == "agent_pane_busy" && pinned.is_some() => {
                    let until = *deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
                    if Instant::now() >= until
                        || self.launch_terminal(session_id, request).await? != pinned
                        || !self.launch_shell_initializing(session_id, request).await?
                    {
                        return Err(error);
                    }
                    tokio::time::sleep(Duration::from_millis(100).min(until.saturating_duration_since(Instant::now()))).await;
                    if Instant::now() >= until
                        || self.launch_terminal(session_id, request).await? != pinned
                        || !self.launch_shell_initializing(session_id, request).await?
                    {
                        return Err(error);
                    }
                }
                Err(error) => return Err(unknown(error)),
            }
        }
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

impl HerdrCliAdapter {
    async fn launch_terminal(
        &self,
        session_id: &str,
        request: &AgentStartRequest,
    ) -> Result<Option<String>, InspectionError> {
        let (result, _) = self
            .socket_request_with_identity(
                session_id,
                "pane.get",
                json!({"pane_id":request.pane_id}),
                Some(&request.endpoint_identity),
            )
            .await?;
        Ok(result
            .get("pane")
            .and_then(|pane| pane.get("terminal_id"))
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    async fn launch_shell_initializing(
        &self,
        session_id: &str,
        request: &AgentStartRequest,
    ) -> Result<bool, InspectionError> {
        let (result, _) = self
            .socket_request_with_identity(
                session_id,
                "pane.process_info",
                json!({"pane_id":request.pane_id}),
                Some(&request.endpoint_identity),
            )
            .await?;
        Ok(process_info_shows_shell_initialization(
            &result["process_info"],
        ))
    }
}

// Source: Herdr v0.9.3 cli/agent.rs and platform/mod.rs. A busy
// foreground child is not evidence of shell initialization.
fn process_info_shows_shell_initialization(info: &Value) -> bool {
    let Some(shell_pid) = info["shell_pid"].as_u64() else {
        return false;
    };
    info["foreground_process_group_id"].as_u64() == Some(shell_pid)
        && info["foreground_processes"]
            .as_array()
            .is_some_and(|processes| {
                processes.iter().any(|process| {
                    process["pid"].as_u64() == Some(shell_pid)
                        && (process["name"].as_str().is_some_and(is_shell_name)
                            || process["argv"]
                                .as_array()
                                .and_then(|argv| argv.first())
                                .and_then(Value::as_str)
                                .is_some_and(is_shell_name))
                })
            })
}

fn is_shell_name(name: &str) -> bool {
    let basename = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_start_matches('-');
    let basename = basename.trim_end_matches(".exe");
    [
        "sh",
        "bash",
        "dash",
        "zsh",
        "fish",
        "ksh",
        "mksh",
        "csh",
        "tcsh",
        "elvish",
        "xonsh",
        "nu",
        "pwsh",
        "powershell",
        "cmd",
    ]
    .iter()
    .any(|shell| basename.eq_ignore_ascii_case(shell))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(
            launch_environment(&BTreeMap::from([(
                "PI_CODING_AGENT_SESSION_DIR".into(),
                "/tmp/fresh".into()
            )]))
            .is_ok()
        );
    }
    #[test]
    fn pending_start_receipt_uses_name_not_manifest_kind() {
        let request = AgentStartRequest {
            endpoint_identity: "endpoint".into(),
            pane_id: "w1:p2".into(),
            name: "ck-run-1-0".into(),
            kind: "omp".into(),
            args: vec![],
            timeout_ms: 60_000,
        };
        let pending = json!({"type":"agent_started","agent":{"name":"ck-run-1-0","pane_id":"w1:p2","launch_pending":true}});
        assert!(validate_start_receipt(&pending, &request).is_ok());
        let detected = json!({"type":"agent_started","agent":{"name":"ck-run-1-0","agent":"omp","pane_id":"w1:p2"}});
        assert!(validate_start_receipt(&detected, &request).is_ok());
        let wrong = json!({"type":"agent_started","agent":{"name":"another-run","agent":"omp","pane_id":"w1:p2"}});
        assert_eq!(
            validate_start_receipt(&wrong, &request).unwrap_err().code,
            "herdr_outcome_unknown"
        );
    }
    #[test]
    fn busy_retry_requires_the_shell_as_foreground_group_owner() {
        let initializing = json!({"shell_pid":12,"foreground_process_group_id":12,"foreground_processes":[{"pid":12,"name":"-zsh","argv":["/bin/zsh"]}]});
        assert!(process_info_shows_shell_initialization(&initializing));
        let child = json!({"shell_pid":12,"foreground_process_group_id":20,"foreground_processes":[{"pid":20,"name":"omp","argv":["omp"]}]});
        assert!(!process_info_shows_shell_initialization(&child));
        let same_group_child = json!({"shell_pid":12,"foreground_process_group_id":12,"foreground_processes":[{"pid":13,"name":"bash","argv":["bash"]}]});
        assert!(!process_info_shows_shell_initialization(&same_group_child));
        assert!(!process_info_shows_shell_initialization(&json!({})));
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
        for code in ["request_outcome_unknown", "response_timeout", "malformed_json", "malformed_response"] {
            let mapped = unknown(InspectionError::new(code, "specific failure"));
            assert_eq!(mapped.code, "herdr_outcome_unknown");
            assert_eq!(mapped.message, "specific failure");
        }
    }
}
