use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc};

use async_trait::async_trait;
use base64::Engine;
use cockpit_core::{
    HerdrAdapter, InspectionError, SessionChange, SessionSubscription, TerminalSession,
};
use cockpit_protocol::v1::{
    AgentSummary, FocusKind, FocusRequest, FocusResponse, HerdrCompatibility, HerdrIdentity,
    LayoutPane, LayoutRect, PaneMoveDestination, PaneResizeDirection, PaneSplitDirection,
    PaneSummary, PaneZoomMode, ResourceMutationRequest, ResourceMutationResponse,
    SessionListResponse, SessionSnapshotResponse, SessionSummary, SpaceGitSummary, SpaceSummary,
    TabLayout, TabSummary, TerminalCommand, TerminalMode, TerminalOpenRequest,
    TerminalOwnershipState, TerminalStreamMessage,
};
use futures_util::future::join_all;
use serde_json::{Value, json};

use crate::schema::{schema_fields, status_fields};
pub const REQUIRED_VERSION: &str = "0.8.2";
pub const REQUIRED_PROTOCOL: u32 = 20;
pub const REQUIRED_SCHEMA_VERSION: u32 = 1;
const REQUIRED_METHODS: [&str; 20] = [
    "ping",
    "session.snapshot",
    "events.subscribe",
    "worktree.list",
    "pane.read",
    "workspace.create",
    "workspace.rename",
    "workspace.move_block",
    "workspace.close",
    "tab.create",
    "tab.rename",
    "tab.move",
    "tab.close",
    "pane.split",
    "pane.resize",
    "pane.rename",
    "pane.swap",
    "pane.move",
    "pane.zoom",
    "pane.close",
];
const MAX_SAFE_REVISION: u64 = 9_007_199_254_740_991;
const MAX_SESSION_NAME: usize = 96;
const MAX_TERMINAL_LINE: usize = 1024 * 1024;
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_STREAM_ID: AtomicU64 = AtomicU64::new(1);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(5);
const SERVER_START_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ConfigError {}

/// Resolved command configuration. `None` means Herdr's own default behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrCliConfig {
    pub executable: PathBuf,
    pub session: Option<String>,
    pub socket: Option<PathBuf>,
}

impl HerdrCliConfig {
    pub fn from_options(
        executable: Option<PathBuf>,
        session: Option<String>,
        socket: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let env: BTreeMap<String, String> = std::env::vars().collect();
        Self::from_options_with_environment(executable, session, socket, &env)
    }

    /// Resolve command options without touching process-global environment state.
    pub fn from_options_with_environment(
        executable: Option<PathBuf>,
        session: Option<String>,
        socket: Option<PathBuf>,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, ConfigError> {
        let executable = executable
            .or_else(|| nonempty(environment.get("COCKPIT_HERDR_EXECUTABLE")).map(PathBuf::from))
            .unwrap_or_else(|| default_herdr_executable(environment));
        let session = session.or_else(|| nonempty(environment.get("COCKPIT_HERDR_SESSION")));
        if let Some(session_name) = session.as_deref()
            && !valid_session_name(session_name)
        {
            return Err(ConfigError {
                code: "invalid_session_name".into(),
                message: "session name contains unsupported characters".into(),
            });
        }
        let socket =
            socket.or_else(|| nonempty(environment.get("COCKPIT_HERDR_SOCKET")).map(PathBuf::from));
        if socket.is_some() && session.is_none() {
            return Err(ConfigError {
                code: "missing_socket_session".into(),
                message: "--herdr-socket/COCKPIT_HERDR_SOCKET requires an explicit --herdr-session/COCKPIT_HERDR_SESSION".into(),
            });
        }
        Ok(Self {
            executable,
            session,
            socket,
        })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }
    pub fn socket(&self) -> Option<&Path> {
        self.socket.as_deref()
    }
}

fn nonempty(value: Option<&String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty()).cloned()
}

fn default_herdr_executable(environment: &BTreeMap<String, String>) -> PathBuf {
    let executable_name = format!("herdr{}", std::env::consts::EXE_SUFFIX);
    if let Some(path) = environment.get("PATH") {
        for directory in std::env::split_paths(path) {
            let candidate = directory.join(&executable_name);
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }

    if let Some(home) = nonempty(environment.get("HOME")).map(PathBuf::from) {
        for relative in [".local/bin", ".linuxbrew/bin", ".cargo/bin"] {
            let candidate = home.join(relative).join(&executable_name);
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }

    for directory in [
        "/home/linuxbrew/.linuxbrew/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
    ] {
        let candidate = Path::new(directory).join(&executable_name);
        if is_executable_file(&candidate) {
            return candidate;
        }
    }

    PathBuf::from(executable_name)
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}
fn malformed(message: impl Into<String>) -> InspectionError {
    InspectionError::new("malformed_json", message)
}

fn server_not_running(status: &Value) -> bool {
    status.get("running").and_then(Value::as_bool) == Some(false)
        || status.get("status").and_then(Value::as_str) == Some("not_running")
}
fn structured_error(value: &Value) -> Result<Option<InspectionError>, InspectionError> {
    let Some(error) = value.get("error") else {
        return Ok(None);
    };
    let error = object(error, "response.error")?;
    Ok(Some(InspectionError::new(
        required_string(error, "code", "response.error")?,
        required_string(error, "message", "response.error")?,
    )))
}
async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut String,
    limit: usize,
) -> std::io::Result<usize> {
    line.clear();
    let mut bytes = Vec::with_capacity(limit.min(4096));
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if bytes.is_empty() {
                return Ok(0);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line is not newline terminated",
            ));
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line exceeds configured limit",
            ));
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "line is not valid UTF-8")
            })?;
            line.push_str(text);
            return Ok(line.len());
        }
    }
}
async fn read_bounded_output<R: AsyncRead + Unpin>(
    reader: R,
    limit: usize,
    context: &str,
) -> Result<Vec<u8>, InspectionError> {
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    let mut reader = reader.take(limit.saturating_add(1) as u64);
    reader.read_to_end(&mut bytes).await.map_err(|_| {
        InspectionError::new("bounded_output", format!("{context} could not be read"))
    })?;
    if bytes.len() > limit {
        return Err(InspectionError::new(
            "bounded_output",
            format!("{context} exceeds configured limit"),
        ));
    }
    Ok(bytes)
}

fn object<'a>(
    value: &'a Value,
    context: &str,
) -> Result<&'a serde_json::Map<String, Value>, InspectionError> {
    value
        .as_object()
        .ok_or_else(|| malformed(format!("{context} must be an object")))
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<String, InspectionError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| malformed(format!("{context}.{key} must be a string")))
}

fn optional_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Option<String>, InspectionError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| malformed(format!("{context}.{key} must be a string or null"))),
    }
}

fn required_u32(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<u32, InspectionError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| malformed(format!("{context}.{key} must be a uint32")))
}

fn required_u64(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<u64, InspectionError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed(format!("{context}.{key} must be a uint64")))
}

fn required_bool(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<bool, InspectionError> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| malformed(format!("{context}.{key} must be a boolean")))
}

fn required_array<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<&'a Vec<Value>, InspectionError> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| malformed(format!("{context}.{key} must be an array")))
}

fn optional_id(
    object: &serde_json::Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Option<String>, InspectionError> {
    optional_string(object, key, context)
}

fn layout_rect(value: &Value, context: &str) -> Result<LayoutRect, InspectionError> {
    let object = object(value, context)?;
    Ok(LayoutRect {
        x: required_u32(object, "x", context)?,
        y: required_u32(object, "y", context)?,
        width: required_u32(object, "width", context)?,
        height: required_u32(object, "height", context)?,
    })
}

fn parse_snapshot(
    value: Value,
    session_id: &str,
) -> Result<SessionSnapshotResponse, InspectionError> {
    let root = object(&value, "response")?;
    let result = root
        .get("result")
        .ok_or_else(|| malformed("response.result is required"))?;
    let result = object(result, "response.result")?;
    if required_string(result, "type", "response.result")? != "session_snapshot" {
        return Err(malformed("response.result.type must be session_snapshot"));
    }
    let snapshot = result
        .get("snapshot")
        .ok_or_else(|| malformed("response.result.snapshot is required"))?;
    let snapshot = object(snapshot, "response.result.snapshot")?;

    let version = required_string(snapshot, "version", "snapshot")?;
    let protocol = required_u32(snapshot, "protocol", "snapshot")?;
    let focused_space_id = optional_id(snapshot, "focused_workspace_id", "snapshot")?;
    let focused_tab_id = optional_id(snapshot, "focused_tab_id", "snapshot")?;
    let focused_pane_id = optional_id(snapshot, "focused_pane_id", "snapshot")?;

    let spaces = required_array(snapshot, "workspaces", "snapshot")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("snapshot.workspaces[{index}]");
            let object = object(value, &context)?;
            Ok(SpaceSummary {
                id: required_string(object, "workspace_id", &context)?,
                label: required_string(object, "label", &context)?,
                number: required_u32(object, "number", &context)?,
                tab_count: required_u32(object, "tab_count", &context)?,
                pane_count: required_u32(object, "pane_count", &context)?,
                focused: required_bool(object, "focused", &context)?,
                agent_status: match object.get("agent_status") {
                    None => "unknown".to_owned(),
                    Some(_) => required_string(object, "agent_status", &context)?,
                },
                git: None,
            })
        })
        .collect::<Result<Vec<_>, InspectionError>>()?;

    let tabs = required_array(snapshot, "tabs", "snapshot")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("snapshot.tabs[{index}]");
            let object = object(value, &context)?;
            Ok(TabSummary {
                id: required_string(object, "tab_id", &context)?,
                space_id: required_string(object, "workspace_id", &context)?,
                label: required_string(object, "label", &context)?,
                number: required_u32(object, "number", &context)?,
                pane_count: required_u32(object, "pane_count", &context)?,
                focused: required_bool(object, "focused", &context)?,
            })
        })
        .collect::<Result<Vec<_>, InspectionError>>()?;

    let panes = required_array(snapshot, "panes", "snapshot")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("snapshot.panes[{index}]");
            let object = object(value, &context)?;
            let title = sanitized_title(object, &context)?;
            let revision = required_u64(object, "revision", &context)?;
            if revision > MAX_SAFE_REVISION {
                return Err(malformed(format!(
                    "{context}.revision exceeds JavaScript safe integer range"
                )));
            }
            Ok(PaneSummary {
                id: required_string(object, "pane_id", &context)?,
                terminal_id: required_string(object, "terminal_id", &context)?,
                space_id: required_string(object, "workspace_id", &context)?,
                tab_id: required_string(object, "tab_id", &context)?,
                title,
                focused: required_bool(object, "focused", &context)?,
                agent: optional_string(object, "agent", &context)?,
                agent_status: required_string(object, "agent_status", &context)?,
                revision,
            })
        })
        .collect::<Result<Vec<_>, InspectionError>>()?;

    let layouts = required_array(snapshot, "layouts", "snapshot")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("snapshot.layouts[{index}]");
            let layout = object(value, &context)?;
            let panes = required_array(layout, "panes", &context)?
                .iter()
                .enumerate()
                .map(|(pane_index, value)| {
                    let pane_context = format!("{context}.panes[{pane_index}]");
                    let pane = object(value, &pane_context)?;
                    Ok(LayoutPane {
                        pane_id: required_string(pane, "pane_id", &pane_context)?,
                        focused: required_bool(pane, "focused", &pane_context)?,
                        rect: layout_rect(
                            pane.get("rect").ok_or_else(|| {
                                malformed(format!("{pane_context}.rect is required"))
                            })?,
                            &format!("{pane_context}.rect"),
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, InspectionError>>()?;
            Ok(TabLayout {
                space_id: required_string(layout, "workspace_id", &context)?,
                tab_id: required_string(layout, "tab_id", &context)?,
                area: layout_rect(
                    layout
                        .get("area")
                        .ok_or_else(|| malformed(format!("{context}.area is required")))?,
                    &format!("{context}.area"),
                )?,
                focused_pane_id: optional_id(layout, "focused_pane_id", &context)?,
                panes,
                zoomed: required_bool(layout, "zoomed", &context)?,
            })
        })
        .collect::<Result<Vec<_>, InspectionError>>()?;

    let agents = required_array(snapshot, "agents", "snapshot")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("snapshot.agents[{index}]");
            let object = object(value, &context)?;
            let title = sanitized_title(object, &context)?;
            let state_change_seq = object
                .get("state_change_seq")
                .map(|_| required_u64(object, "state_change_seq", &context))
                .transpose()?
                .unwrap_or_default();
            if state_change_seq > MAX_SAFE_REVISION {
                return Err(malformed(format!(
                    "{context}.state_change_seq exceeds JavaScript safe integer range"
                )));
            }
            Ok(AgentSummary {
                pane_id: required_string(object, "pane_id", &context)?,
                space_id: required_string(object, "workspace_id", &context)?,
                tab_id: required_string(object, "tab_id", &context)?,
                name: required_string(object, "agent", &context)?,
                status: required_string(object, "agent_status", &context)?,
                title,
                focused: required_bool(object, "focused", &context)?,
                state_change_seq,
            })
        })
        .collect::<Result<Vec<_>, InspectionError>>()?;

    let response = SessionSnapshotResponse {
        session_id: session_id.to_owned(),
        version,
        protocol,
        focused_space_id,
        focused_tab_id,
        focused_pane_id,
        spaces,
        tabs,
        panes,
        layouts,
        agents,
    };
    validate_snapshot(&response)?;
    Ok(response)
}
fn parse_space_git_summary(
    value: &Value,
    workspace_id: &str,
) -> Result<Option<SpaceGitSummary>, InspectionError> {
    let result = object(value, "worktree.list result")?;
    if required_string(result, "type", "worktree.list result")? != "worktree_list" {
        return Err(malformed("worktree.list result.type must be worktree_list"));
    }
    let source = result
        .get("source")
        .ok_or_else(|| malformed("worktree.list result.source is required"))?;
    let source = object(source, "worktree.list result.source")?;
    let repository_key = required_string(source, "repo_key", "worktree.list result.source")?;
    let repository = required_string(source, "repo_name", "worktree.list result.source")?;
    let worktrees = required_array(result, "worktrees", "worktree.list result")?;

    for (index, value) in worktrees.iter().enumerate() {
        let context = format!("worktree.list result.worktrees[{index}]");
        let worktree = object(value, &context)?;
        if optional_string(worktree, "open_workspace_id", &context)?.as_deref()
            != Some(workspace_id)
        {
            continue;
        }
        let detached = required_bool(worktree, "is_detached", &context)?;
        return Ok(Some(SpaceGitSummary {
            repository_key,
            repository,
            branch: if detached {
                None
            } else {
                optional_string(worktree, "branch", &context)?
            },
            checkout_path: required_string(worktree, "path", &context)?,
            is_linked_worktree: required_bool(worktree, "is_linked_worktree", &context)?,
        }));
    }
    Ok(None)
}

fn validate_snapshot(snapshot: &SessionSnapshotResponse) -> Result<(), InspectionError> {
    let spaces = &snapshot.spaces;
    let tabs = &snapshot.tabs;
    let panes = &snapshot.panes;
    let layouts = &snapshot.layouts;
    let agents = &snapshot.agents;
    let focused_space_id = snapshot.focused_space_id.as_deref();
    let focused_tab_id = snapshot.focused_tab_id.as_deref();
    let focused_pane_id = snapshot.focused_pane_id.as_deref();
    let mut spaces_by_id = BTreeMap::new();
    for space in spaces {
        if spaces_by_id.insert(space.id.as_str(), space).is_some() {
            return Err(malformed(format!(
                "snapshot.workspaces has duplicate workspace_id {}",
                space.id
            )));
        }
    }

    let mut tabs_by_id = BTreeMap::new();
    for tab in tabs {
        if !spaces_by_id.contains_key(tab.space_id.as_str()) {
            return Err(malformed(format!(
                "snapshot.tabs[{}] references unknown workspace_id {}",
                tab.id, tab.space_id
            )));
        }
        if tabs_by_id.insert(tab.id.as_str(), tab).is_some() {
            return Err(malformed(format!(
                "snapshot.tabs has duplicate tab_id {}",
                tab.id
            )));
        }
    }

    let mut panes_by_id = BTreeMap::new();
    for pane in panes {
        let tab = tabs_by_id.get(pane.tab_id.as_str()).ok_or_else(|| {
            malformed(format!(
                "snapshot.panes[{}] references unknown tab_id {}",
                pane.id, pane.tab_id
            ))
        })?;
        if tab.space_id != pane.space_id {
            return Err(malformed(format!(
                "snapshot.panes[{}] workspace_id does not match tab {}",
                pane.id, pane.tab_id
            )));
        }
        if panes_by_id.insert(pane.id.as_str(), pane).is_some() {
            return Err(malformed(format!(
                "snapshot.panes has duplicate pane_id {}",
                pane.id
            )));
        }
    }

    let mut layout_ids = BTreeSet::new();
    for layout in layouts {
        let layout_key = (layout.space_id.as_str(), layout.tab_id.as_str());
        if !layout_ids.insert(layout_key) {
            return Err(malformed(format!(
                "snapshot.layouts has duplicate layout for tab_id {}",
                layout.tab_id
            )));
        }
        let tab = tabs_by_id.get(layout.tab_id.as_str()).ok_or_else(|| {
            malformed(format!(
                "snapshot.layouts[{}] references unknown tab_id {}",
                layout.tab_id, layout.tab_id
            ))
        })?;
        if tab.space_id != layout.space_id {
            return Err(malformed(format!(
                "snapshot.layouts[{}] workspace_id does not match tab",
                layout.tab_id
            )));
        }
        let mut layout_pane_ids = BTreeSet::new();
        for layout_pane in &layout.panes {
            if !layout_pane_ids.insert(layout_pane.pane_id.as_str()) {
                return Err(malformed(format!(
                    "snapshot.layouts[{}] has duplicate pane_id {}",
                    layout.tab_id, layout_pane.pane_id
                )));
            }
            let pane = panes_by_id
                .get(layout_pane.pane_id.as_str())
                .ok_or_else(|| {
                    malformed(format!(
                        "snapshot.layouts[{}] references unknown pane_id {}",
                        layout.tab_id, layout_pane.pane_id
                    ))
                })?;
            if pane.space_id != layout.space_id || pane.tab_id != layout.tab_id {
                return Err(malformed(format!(
                    "snapshot.layouts[{}] pane_id {} does not match layout",
                    layout.tab_id, layout_pane.pane_id
                )));
            }
        }
        if let Some(focused_pane_id) = &layout.focused_pane_id
            && !layout_pane_ids.contains(focused_pane_id.as_str())
        {
            return Err(malformed(format!(
                "snapshot.layouts[{}].focused_pane_id {} is not in layout",
                layout.tab_id, focused_pane_id
            )));
        }
    }

    for agent in agents {
        let pane = panes_by_id.get(agent.pane_id.as_str()).ok_or_else(|| {
            malformed(format!(
                "snapshot.agents[{}] references unknown pane_id {}",
                agent.pane_id, agent.pane_id
            ))
        })?;
        if pane.space_id != agent.space_id || pane.tab_id != agent.tab_id {
            return Err(malformed(format!(
                "snapshot.agents[{}] location does not match pane",
                agent.pane_id
            )));
        }
    }

    if let Some(focused_space_id) = focused_space_id {
        let space = spaces_by_id.get(focused_space_id).ok_or_else(|| {
            malformed(format!(
                "focused_workspace_id {} is not in workspaces",
                focused_space_id
            ))
        })?;
        if !space.focused {
            return Err(malformed(format!(
                "focused_workspace_id {} is not marked focused",
                focused_space_id
            )));
        }
    } else if focused_tab_id.is_some() || focused_pane_id.is_some() {
        return Err(malformed("focused tab or pane requires focused workspace"));
    }

    if let Some(focused_tab_id) = focused_tab_id {
        let tab = tabs_by_id.get(focused_tab_id).ok_or_else(|| {
            malformed(format!("focused_tab_id {} is not in tabs", focused_tab_id))
        })?;
        if !tab.focused {
            return Err(malformed(format!(
                "focused_tab_id {} is not marked focused",
                focused_tab_id
            )));
        }
        if Some(tab.space_id.as_str()) != focused_space_id {
            return Err(malformed(format!(
                "focused_tab_id {} does not belong to focused workspace",
                focused_tab_id
            )));
        }
    }

    if let Some(focused_pane_id) = focused_pane_id {
        let pane = panes_by_id.get(focused_pane_id).ok_or_else(|| {
            malformed(format!(
                "focused_pane_id {} is not in panes",
                focused_pane_id
            ))
        })?;
        if !pane.focused {
            return Err(malformed(format!(
                "focused_pane_id {} is not marked focused",
                focused_pane_id
            )));
        }
        if Some(pane.tab_id.as_str()) != focused_tab_id
            || Some(pane.space_id.as_str()) != focused_space_id
        {
            return Err(malformed(format!(
                "focused_pane_id {} does not belong to focused tab/workspace",
                focused_pane_id
            )));
        }
    }
    Ok(())
}

fn sanitize_terminal_text(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character == '\u{1b}' {
            index += 1;
            let Some(kind) = chars.get(index).copied() else {
                break;
            };
            index += 1;
            match kind {
                '[' => {
                    while index < chars.len() {
                        let character = chars[index];
                        index += 1;
                        if ('@'..='~').contains(&character) {
                            break;
                        }
                    }
                }
                ']' | 'P' | 'X' | '^' | '_' => {
                    while index < chars.len() {
                        let character = chars[index];
                        index += 1;
                        if character == '\u{07}' {
                            break;
                        }
                        if character == '\u{1b}' && chars.get(index) == Some(&'\\') {
                            index += 1;
                            break;
                        }
                    }
                }
                kind if kind.is_ascii() && (' '..='/').contains(&kind) => {
                    while index < chars.len() {
                        let character = chars[index];
                        index += 1;
                        if character.is_ascii() && ('0'..='~').contains(&character) {
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        index += 1;
        match character {
            '\r' => {
                if chars.get(index) == Some(&'\n') {
                    index += 1;
                }
                output.push('\n');
            }
            '\n' | '\t' => output.push(character),
            character if !character.is_control() => output.push(character),
            _ => {}
        }
    }
    output
}

fn sanitized_title(
    object: &serde_json::Map<String, Value>,
    context: &str,
) -> Result<Option<String>, InspectionError> {
    let label =
        optional_string(object, "label", context)?.map(|title| sanitize_terminal_text(&title));
    let stripped = optional_string(object, "terminal_title_stripped", context)?
        .map(|title| sanitize_terminal_text(&title));
    let terminal = optional_string(object, "terminal_title", context)?
        .map(|title| sanitize_terminal_text(&title));
    Ok(label
        .filter(|title| !title.is_empty())
        .or_else(|| stripped.filter(|title| !title.is_empty()))
        .or_else(|| terminal.filter(|title| !title.is_empty())))
}

#[derive(Debug, Clone)]
pub struct HerdrCliAdapter {
    config: HerdrCliConfig,
    autostart_server: bool,
    server_start: Arc<Mutex<()>>,
}

fn valid_pane_id(pane_id: &str) -> bool {
    !pane_id.is_empty()
        && pane_id.len() <= 128
        && pane_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
}

fn valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SESSION_NAME
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventConnectionEnd {
    StreamEnded,
    TopologyChanged,
}

impl HerdrCliAdapter {
    pub fn new(config: HerdrCliConfig) -> Self {
        Self {
            config,
            autostart_server: false,
            server_start: Arc::new(Mutex::new(())),
        }
    }

    pub fn with_server_autostart(mut self) -> Self {
        self.autostart_server = true;
        self
    }

    fn should_start_server(&self, error: &InspectionError) -> bool {
        self.autostart_server
            && self.config.socket().is_none()
            && matches!(
                error.code.as_str(),
                "execution_failed" | "server_not_running"
            )
    }

    pub fn config(&self) -> &HerdrCliConfig {
        &self.config
    }

    fn command(&self, session: Option<&str>, args: &[&str]) -> Command {
        let mut command = Command::new(&self.config.executable);
        command
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_SOCKET_PATH");
        let selected = if self.config.socket.is_some() {
            None
        } else {
            session.or(self.config.session.as_deref())
        };
        if let Some(name) = selected {
            command.arg("--session").arg(name);
        }
        command.args(args);
        if let Some(socket) = &self.config.socket {
            command.env("HERDR_SOCKET_PATH", socket);
        }
        command
    }

    async fn run_json_for(
        &self,
        session: Option<&str>,
        args: &[&str],
    ) -> Result<Value, InspectionError> {
        let mut child = self
            .command(session, args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                InspectionError::new(
                    "execution_failed",
                    format!("failed to execute Herdr: {error}"),
                )
            })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            InspectionError::new("execution_failed", "Herdr stdout was not captured")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            InspectionError::new("execution_failed", "Herdr stderr was not captured")
        })?;
        let (stdout, stderr) = tokio::join!(
            read_bounded_output(stdout, MAX_TERMINAL_LINE, "Herdr stdout"),
            read_bounded_output(stderr, 16 * 1024, "Herdr stderr")
        );
        if let Err(error) = stdout.as_ref() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error.clone());
        }
        if let Err(error) = stderr.as_ref() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error.clone());
        }
        let status = child.wait().await.map_err(|_| {
            InspectionError::new("execution_failed", "Herdr command did not finish")
        })?;
        if !status.success() {
            return Err(InspectionError::new(
                "execution_failed",
                format!("Herdr command failed with status {status}"),
            ));
        }
        let value =
            serde_json::from_slice(stdout.as_ref().expect("checked stdout")).map_err(|error| {
                InspectionError::new(
                    "malformed_json",
                    format!("Herdr returned invalid JSON: {error}"),
                )
            })?;
        if let Some(error) = structured_error(&value)? {
            return Err(error);
        }
        Ok(value)
    }

    fn selected_session(&self, session_id: &str) -> Result<(), InspectionError> {
        if !valid_session_name(session_id) {
            return Err(InspectionError::new(
                "invalid_session_id",
                "session ID contains unsupported characters",
            ));
        }
        if let Some(configured) = self.config.session()
            && configured != session_id
        {
            return Err(InspectionError::new(
                "session_not_selected",
                "adapter endpoint is bound to another session",
            ));
        }
        if self.config.socket.is_some() && self.config.session().is_none() {
            return Err(InspectionError::new(
                "session_not_selected",
                "custom socket endpoint has no configured session identity",
            ));
        }
        Ok(())
    }

    fn socket_path(&self, session_id: &str) -> Result<PathBuf, InspectionError> {
        self.selected_session(session_id)?;
        if let Some(path) = self.config.socket() {
            return Ok(path.to_owned());
        }
        let config_dir = std::env::var_os("HERDR_CONFIG_DIR")
            .or_else(|| {
                std::env::var_os("HERDR_CONFIG_PATH").and_then(|path| {
                    PathBuf::from(path)
                        .parent()
                        .map(|parent| parent.as_os_str().to_owned())
                })
            })
            .or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(|p| PathBuf::from(p).join("herdr").into_os_string())
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|p| PathBuf::from(p).join(".config/herdr").into_os_string())
            })
            .map(PathBuf::from)
            .ok_or_else(|| {
                InspectionError::new(
                    "endpoint_unavailable",
                    "Herdr config directory is unavailable",
                )
            })?;
        if session_id == "default" {
            Ok(config_dir.join("herdr.sock"))
        } else {
            Ok(config_dir
                .join("sessions")
                .join(session_id)
                .join("herdr.sock"))
        }
    }

    async fn socket_request(
        &self,
        session_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, InspectionError> {
        let path = self.socket_path(session_id)?;
        let mut stream = UnixStream::connect(path).await.map_err(|_error| {
            InspectionError::new("connection_failed", "Herdr connection failed")
        })?;
        let id = format!(
            "cockpit-{}",
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );
        let request = json!({"id": id, "method": method, "params": params});
        let mut line = serde_json::to_vec(&request)
            .map_err(|error| InspectionError::new("malformed_json", error.to_string()))?;
        line.push(b'\n');
        stream.write_all(&line).await.map_err(|error| {
            InspectionError::new(
                "connection_failed",
                format!("Herdr request failed: {error}"),
            )
        })?;
        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        loop {
            response.clear();
            let read = read_bounded_line(&mut reader, &mut response, MAX_TERMINAL_LINE)
                .await
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::InvalidData {
                        InspectionError::new(
                            "bounded_output",
                            "Herdr response line exceeds configured limit or is unterminated",
                        )
                    } else {
                        InspectionError::new("connection_failed", "Herdr response failed")
                    }
                })?;
            if read == 0 {
                return Err(InspectionError::new(
                    "disconnected",
                    "Herdr closed the connection",
                ));
            }
            if response.len() > MAX_TERMINAL_LINE {
                return Err(malformed("Herdr response line is too large"));
            }
            let value: Value = serde_json::from_str(response.trim_end())
                .map_err(|error| malformed(format!("Herdr returned invalid JSON: {error}")))?;
            if value.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if let Some(error) = structured_error(&value)? {
                return Err(error);
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| malformed("Herdr response.result is required"));
        }
    }

    async fn start_server_and_wait(&self, session: Option<&str>) -> Result<Value, InspectionError> {
        let mut child = self
            .command(session, &["server"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                InspectionError::new(
                    "server_start_failed",
                    format!("failed to start Herdr server: {error}"),
                )
            })?;
        let deadline = tokio::time::Instant::now() + SERVER_START_TIMEOUT;
        let mut last_error: InspectionError;

        loop {
            tokio::time::sleep(SERVER_START_POLL_INTERVAL).await;
            match self
                .run_json_for(session, &["status", "server", "--json"])
                .await
            {
                Ok(status) if !server_not_running(&status) => {
                    tokio::spawn(async move {
                        let _ = child.wait().await;
                    });
                    return Ok(status);
                }
                Ok(_) => {
                    last_error =
                        InspectionError::new("server_not_running", "Herdr server is not running");
                }
                Err(error) => last_error = error,
            }
            if let Some(status) = child.try_wait().map_err(|error| {
                InspectionError::new(
                    "server_start_failed",
                    format!("could not monitor Herdr server startup: {error}"),
                )
            })? {
                return Err(InspectionError::new(
                    "server_start_failed",
                    format!(
                        "Herdr server exited before becoming ready ({status}); {}",
                        last_error.message
                    ),
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(InspectionError::new(
                    "server_start_timeout",
                    format!(
                        "Herdr server did not become ready within {} seconds; {}",
                        SERVER_START_TIMEOUT.as_secs(),
                        last_error.message
                    ),
                ));
            }
        }
    }

    async fn inspect_with_server_autostart(
        &self,
        session: Option<&str>,
    ) -> Result<HerdrCompatibility, InspectionError> {
        let initial = match self.inspect_inner(session).await {
            Ok(compatibility) => return Ok(compatibility),
            Err(error) => error,
        };
        if !self.should_start_server(&initial) {
            return Err(initial);
        }

        let _start_guard = self.server_start.lock().await;
        match self.inspect_inner(session).await {
            Ok(compatibility) => Ok(compatibility),
            Err(error) if self.should_start_server(&error) => {
                let status = self.start_server_and_wait(session).await?;
                self.inspect_status(session, status).await
            }
            Err(error) => Err(error),
        }
    }

    async fn inspect_inner(
        &self,
        session: Option<&str>,
    ) -> Result<HerdrCompatibility, InspectionError> {
        let status = self
            .run_json_for(session, &["status", "server", "--json"])
            .await?;
        if server_not_running(&status) {
            return Err(InspectionError::new(
                "server_not_running",
                "Herdr server is not running",
            ));
        }
        self.inspect_status(session, status).await
    }

    async fn inspect_status(
        &self,
        session: Option<&str>,
        status: Value,
    ) -> Result<HerdrCompatibility, InspectionError> {
        let (version, protocol) = status_fields(&status)
            .ok_or_else(|| malformed("Herdr status omitted top-level version or protocol"))?;
        if version != REQUIRED_VERSION {
            return Ok(Self::incompatible(
                None,
                "version_mismatch",
                format!("expected Herdr {REQUIRED_VERSION}"),
            ));
        }
        if protocol != REQUIRED_PROTOCOL {
            return Ok(Self::incompatible(
                None,
                "protocol_mismatch",
                format!("expected Herdr protocol {REQUIRED_PROTOCOL}"),
            ));
        }
        let schema = self
            .run_json_for(session, &["api", "schema", "--json"])
            .await?;
        let (schema_version, methods) = schema_fields(&schema).ok_or_else(|| {
            malformed("Herdr schema omitted top-level schema_version or request declarations")
        })?;
        let identity = HerdrIdentity {
            version,
            protocol,
            schema_version,
        };
        if schema_version != REQUIRED_SCHEMA_VERSION {
            return Ok(Self::incompatible(
                Some(identity),
                "schema_version_mismatch",
                format!("expected schema version {REQUIRED_SCHEMA_VERSION}"),
            ));
        }
        let missing: Vec<&str> = REQUIRED_METHODS
            .iter()
            .copied()
            .filter(|method| !methods.contains(*method))
            .collect();
        if !missing.is_empty() {
            return Ok(Self::incompatible(
                Some(identity),
                "missing_methods",
                format!("required Herdr methods are missing: {}", missing.join(", ")),
            ));
        }
        Ok(HerdrCompatibility::Compatible { identity })
    }

    fn incompatible(
        identity: Option<HerdrIdentity>,
        code: &str,
        message: impl Into<String>,
    ) -> HerdrCompatibility {
        HerdrCompatibility::Incompatible {
            identity,
            code: code.to_owned(),
            message: message.into(),
        }
    }

    async fn read_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError> {
        self.selected_session(session_id)?;
        let result = self
            .socket_request(session_id, "session.snapshot", json!({}))
            .await?;
        let mut snapshot = parse_snapshot(json!({"result": result}), session_id)?;
        let git_summaries = join_all(snapshot.spaces.iter().map(|space| async {
            match self
                .socket_request(
                    session_id,
                    "worktree.list",
                    json!({"workspace_id": space.id}),
                )
                .await
            {
                Ok(result) => parse_space_git_summary(&result, &space.id).ok().flatten(),
                Err(_) => None,
            }
        }))
        .await;
        for (space, git) in snapshot.spaces.iter_mut().zip(git_summaries) {
            space.git = git;
        }
        Ok(snapshot)
    }
    async fn mutate_resource(
        &self,
        session_id: &str,
        request: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError> {
        self.selected_session(session_id)?;
        let (method, params) = mutation_call(request);
        drop(self.socket_request(session_id, method, params).await?);
        let snapshot = self.read_snapshot(session_id).await.map_err(|error| {
            InspectionError::new(
                "mutation_applied_snapshot_failed",
                format!(
                    "the mutation may already be applied; only resync is safe because the authoritative snapshot refresh failed ({}): {}",
                    error.code, error.message
                ),
            )
        })?;
        Ok(ResourceMutationResponse {
            session_id: session_id.to_owned(),
            snapshot,
        })
    }

    async fn subscribe_socket(
        &self,
        session_id: String,
        snapshot: SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError> {
        self.selected_session(&session_id)?;
        if snapshot.session_id != session_id {
            return Err(InspectionError::new(
                "session_mismatch",
                "snapshot belongs to another session",
            ));
        }
        let (sender, receiver) = mpsc::channel(32);
        let (ready_sender, mut ready_receiver) = mpsc::channel(1);
        let adapter = self.clone();
        tokio::spawn(async move {
            let mut delay_ms = 100_u64;
            let mut current_snapshot = snapshot;
            let mut initial_ready = Some(ready_sender);
            loop {
                if sender.is_closed() {
                    break;
                }
                let result = adapter
                    .event_connection(
                        &session_id,
                        &current_snapshot,
                        &sender,
                        initial_ready.take(),
                    )
                    .await;
                if sender.is_closed() {
                    break;
                }
                let (change, retry) = match result {
                    Ok(EventConnectionEnd::TopologyChanged) => {
                        loop {
                            match adapter.read_snapshot(&session_id).await {
                                Ok(new_snapshot) => {
                                    current_snapshot = new_snapshot;
                                    delay_ms = 100;
                                    break;
                                }
                                Err(error) => {
                                    if sender
                                        .send(SessionChange::Stale {
                                            code: error.code,
                                            message: error.message,
                                        })
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                    delay_ms = (delay_ms.saturating_mul(2)).min(2000);
                                }
                            }
                        }
                        continue;
                    }
                    Ok(EventConnectionEnd::StreamEnded) => (
                        SessionChange::Disconnected {
                            code: "disconnected".into(),
                            message: "Herdr event stream ended".into(),
                        },
                        true,
                    ),
                    Err(error) => {
                        let change = if error.code == "malformed_json"
                            || error.code == "malformed_event"
                            || error.code == "unknown_event"
                            || error.code == "bounded_output"
                        {
                            SessionChange::Stale {
                                code: error.code,
                                message: error.message,
                            }
                        } else {
                            SessionChange::Disconnected {
                                code: error.code,
                                message: error.message,
                            }
                        };
                        (change, true)
                    }
                };
                if sender.send(change).await.is_err() {
                    break;
                }
                if !retry {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                delay_ms = (delay_ms.saturating_mul(2)).min(2000);
                match adapter.read_snapshot(&session_id).await {
                    Ok(new_snapshot) => {
                        current_snapshot = new_snapshot;
                        if sender.send(SessionChange::Changed).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        if sender
                            .send(SessionChange::Stale {
                                code: error.code,
                                message: error.message,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        match ready_receiver.recv().await {
            Some(Ok(())) => Ok(SessionSubscription { messages: receiver }),
            Some(Err(error)) => Err(error),
            None => Err(InspectionError::new(
                "subscription_setup_failed",
                "Herdr event subscription setup ended",
            )),
        }
    }

    async fn event_connection(
        &self,
        session_id: &str,
        snapshot: &SessionSnapshotResponse,
        sender: &mpsc::Sender<SessionChange>,
        readiness: Option<mpsc::Sender<Result<(), InspectionError>>>,
    ) -> Result<EventConnectionEnd, InspectionError> {
        let result = self
            .event_connection_inner(session_id, snapshot, sender, readiness.clone())
            .await;
        if let Some(readiness) = readiness {
            let result = Err(result.as_ref().err().cloned().unwrap_or_else(|| {
                InspectionError::new(
                    "subscription_setup_failed",
                    "Herdr event stream ended before setup completed",
                )
            }));
            let _ = readiness.send(result).await;
        }
        result
    }

    async fn event_connection_inner(
        &self,
        session_id: &str,
        snapshot: &SessionSnapshotResponse,
        sender: &mpsc::Sender<SessionChange>,
        readiness: Option<mpsc::Sender<Result<(), InspectionError>>>,
    ) -> Result<EventConnectionEnd, InspectionError> {
        let path = self.socket_path(session_id)?;
        let mut stream = UnixStream::connect(path).await.map_err(|_error| {
            InspectionError::new("connection_failed", "Herdr connection failed")
        })?;
        let subscriptions = event_subscriptions(snapshot);
        let id = format!(
            "cockpit-sub-{}",
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );
        let request = json!({"id": id, "method":"events.subscribe", "params":{"subscriptions":subscriptions}});
        let mut bytes =
            serde_json::to_vec(&request).map_err(|error| malformed(error.to_string()))?;
        bytes.push(b'\n');
        stream
            .write_all(&bytes)
            .await
            .map_err(|error| InspectionError::new("connection_failed", error.to_string()))?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let mut acknowledged = false;
        loop {
            let n = read_bounded_line(&mut reader, &mut line, MAX_TERMINAL_LINE)
                .await
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::InvalidData {
                        InspectionError::new(
                            "bounded_output",
                            "Herdr event line exceeds configured limit or is unterminated",
                        )
                    } else {
                        InspectionError::new("disconnected", "Herdr event stream read failed")
                    }
                })?;
            if n == 0 {
                return Ok(EventConnectionEnd::StreamEnded);
            }
            let value: Value = serde_json::from_str(line.trim_end()).map_err(|error| {
                InspectionError::new("malformed_event", format!("invalid event JSON: {error}"))
            })?;
            if !acknowledged
                && value.get("error").is_some()
                && value
                    .get("id")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            {
                return Err(structured_error(&value)?.unwrap_or_else(|| {
                    InspectionError::new(
                        "subscription_setup_failed",
                        "events.subscribe returned an invalid error",
                    )
                }));
            }
            if let Some(response_id) = value.get("id").and_then(Value::as_str) {
                if response_id != id {
                    continue;
                }
                if let Some(error) = structured_error(&value)? {
                    return Err(error);
                }
                let result = value.get("result").ok_or_else(|| {
                    InspectionError::new(
                        "malformed_event",
                        "subscription response.result is required",
                    )
                })?;
                if result.get("type").and_then(Value::as_str) != Some("subscription_started") {
                    return Err(InspectionError::new(
                        "malformed_event",
                        "events.subscribe acknowledgement is invalid",
                    ));
                }
                acknowledged = true;
                if let Some(readiness) = readiness.as_ref() {
                    let _ = readiness.send(Ok(())).await;
                }
                continue;
            }
            if !acknowledged {
                return Err(InspectionError::new(
                    "malformed_event",
                    "event arrived before subscription acknowledgement",
                ));
            }
            let Some(event) = value.get("event").and_then(Value::as_str) else {
                return Err(InspectionError::new(
                    "malformed_event",
                    "event envelope is invalid",
                ));
            };
            let Some(data) = value.get("data").filter(|data| data.is_object()) else {
                return Err(InspectionError::new(
                    "malformed_event",
                    "event envelope is invalid",
                ));
            };
            if !known_event(event, data, snapshot) {
                return Err(InspectionError::new(
                    "unknown_event",
                    format!(
                        "unknown Herdr event type: {}",
                        sanitize_terminal_text(event)
                    ),
                ));
            }
            if sender.send(SessionChange::Changed).await.is_err() {
                return Ok(EventConnectionEnd::StreamEnded);
            }
            if pane_topology_event(event) {
                return Ok(EventConnectionEnd::TopologyChanged);
            }
        }
    }
    async fn list_sessions(&self) -> Result<SessionListResponse, InspectionError> {
        let value = self
            .run_json_for(None, &["session", "list", "--json"])
            .await?;
        let array = value
            .get("sessions")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("session list omitted sessions"))?;
        let mut sessions = Vec::with_capacity(array.len());
        for (index, item) in array.iter().enumerate() {
            let object = object(item, &format!("sessions[{index}]"))?;
            let name = object
                .get("name")
                .or_else(|| object.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| malformed(format!("sessions[{index}].name must be a string")))?;
            if !valid_session_name(name)
                || self
                    .config
                    .session
                    .as_deref()
                    .is_some_and(|configured| configured != name)
            {
                continue;
            }
            let running = object
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let is_default = object
                .get("default")
                .and_then(Value::as_bool)
                .or_else(|| object.get("is_default").and_then(Value::as_bool))
                .unwrap_or(name == "default");
            sessions.push(SessionSummary {
                id: name.to_owned(),
                label: name.to_owned(),
                is_default,
                running,
            });
        }
        Ok(SessionListResponse { sessions })
    }

    async fn open_terminal_inner(
        &self,
        request: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError> {
        self.selected_session(&request.session_id)?;
        if !valid_pane_id(&request.pane_id) {
            return Err(InspectionError::new(
                "invalid_pane_id",
                "pane ID contains unsupported characters",
            ));
        }
        if request.cols == 0 || request.rows == 0 {
            return Err(InspectionError::new(
                "invalid_terminal_dimensions",
                "terminal dimensions must be greater than zero",
            ));
        }
        let cols = request.cols.to_string();
        let rows = request.rows.to_string();
        let mut args = vec![
            "terminal",
            "session",
            match request.mode {
                TerminalMode::Observe => "observe",
                TerminalMode::Control => "control",
            },
            request.pane_id.as_str(),
            "--cols",
            cols.as_str(),
            "--rows",
            rows.as_str(),
        ];
        if request.mode == TerminalMode::Control && request.takeover {
            args.push("--takeover");
        }
        let mut command = self.command(Some(&request.session_id), &args);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            InspectionError::new(
                "terminal_attach_failed",
                format!("failed to execute Herdr terminal session: {error}"),
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            InspectionError::new("terminal_attach_failed", "terminal stdout unavailable")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            InspectionError::new("terminal_attach_failed", "terminal stderr unavailable")
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            InspectionError::new("terminal_attach_failed", "terminal stdin unavailable")
        })?;
        let (sender, receiver) = mpsc::channel(64);
        let (commands, mut command_receiver) = mpsc::channel(32);
        let stream_id = format!(
            "terminal-{}",
            NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed)
        );
        let task_context = TerminalTaskContext {
            session_id: request.session_id.clone(),
            pane_id: request.pane_id.clone(),
            stream_id: stream_id.clone(),
            mode: request.mode,
        };
        tokio::spawn(async move {
            terminal_task(
                &mut child,
                stdout,
                stderr,
                stdin,
                &mut command_receiver,
                sender,
                task_context,
            )
            .await;
        });
        Ok(TerminalSession {
            stream_id,
            messages: receiver,
            commands,
        })
    }
}

fn event_subscriptions(snapshot: &SessionSnapshotResponse) -> Vec<Value> {
    const TYPES: &[&str] = &[
        "workspace.created",
        "workspace.updated",
        "workspace.metadata_updated",
        "workspace.renamed",
        "workspace.moved",
        "workspace.reordered",
        "workspace.closed",
        "workspace.focused",
        "worktree.created",
        "worktree.opened",
        "worktree.removed",
        "tab.created",
        "tab.closed",
        "tab.focused",
        "tab.renamed",
        "tab.moved",
        "pane.created",
        "pane.closed",
        "pane.updated",
        "pane.focused",
        "pane.moved",
        "pane.exited",
        "pane.agent_detected",
        "layout.updated",
    ];
    let mut result: Vec<Value> = TYPES.iter().map(|kind| json!({"type":kind})).collect();
    for pane in &snapshot.panes {
        result.push(json!({"type":"pane.agent_status_changed","pane_id":pane.id}));
        result.push(json!({"type":"pane.scroll_changed","pane_id":pane.id}));
    }
    result
}

fn pane_topology_event(event: &str) -> bool {
    matches!(
        event,
        "pane.created"
            | "pane_created"
            | "pane.closed"
            | "pane_closed"
            | "pane.moved"
            | "pane_moved"
    )
}

fn known_event(event: &str, data: &Value, snapshot: &SessionSnapshotResponse) -> bool {
    const LIFECYCLE: &[&str] = &[
        "workspace_created",
        "workspace_updated",
        "workspace_metadata_updated",
        "workspace_renamed",
        "workspace_moved",
        "workspace_reordered",
        "workspace_closed",
        "workspace_focused",
        "worktree_created",
        "worktree_opened",
        "worktree_removed",
        "tab_created",
        "tab_closed",
        "tab_focused",
        "tab_renamed",
        "tab_moved",
        "pane_created",
        "pane_closed",
        "pane_updated",
        "pane_focused",
        "pane_moved",
        "pane_exited",
        "pane_output_matched",
        "layout_updated",
    ];
    let normalized = event.replace('.', "_");
    if LIFECYCLE.contains(&normalized.as_str()) {
        return true;
    }
    if normalized == "pane_agent_status_changed" || normalized == "pane_scroll_changed" {
        return data
            .get("pane_id")
            .and_then(Value::as_str)
            .is_some_and(|pane_id| snapshot.panes.iter().any(|pane| pane.id == pane_id));
    }
    false
}

struct TerminalTaskContext {
    session_id: String,
    pane_id: String,
    stream_id: String,
    mode: TerminalMode,
}
async fn terminal_task(
    child: &mut Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    mut stdin: ChildStdin,
    commands: &mut mpsc::Receiver<TerminalCommand>,
    sender: mpsc::Sender<TerminalStreamMessage>,
    context: TerminalTaskContext,
) {
    let TerminalTaskContext {
        session_id,
        pane_id,
        stream_id,
        mode,
    } = context;
    let mut output = BufReader::new(stdout);
    let mut output_line = String::new();
    let stderr_reader = BufReader::new(stderr);
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::with_capacity(16 * 1024);
        let mut limited = stderr_reader.take((16 * 1024 + 1) as u64);
        limited.read_to_end(&mut bytes).await.map_err(|_| {
            InspectionError::new("bounded_output", "Herdr terminal diagnostics failed")
        })?;
        if bytes.len() > 16 * 1024 {
            return Err(InspectionError::new(
                "bounded_output",
                "Herdr terminal diagnostics exceed configured limit",
            ));
        }
        Ok::<String, InspectionError>(String::from_utf8_lossy(&bytes).into_owned())
    });
    let mut previous_seq: Option<u64> = None;
    let mut closed = false;
    let mut ended_with_error = false;
    let mut ownership_announced = mode == TerminalMode::Observe;
    let mut ownership = if mode == TerminalMode::Observe {
        TerminalOwnershipState::Observing
    } else {
        TerminalOwnershipState::Pending
    };
    let _ = sender
        .send(TerminalStreamMessage::Ownership {
            session_id: session_id.clone(),
            pane_id: pane_id.clone(),
            stream_id: stream_id.clone(),
            state: ownership,
            message: None,
        })
        .await;
    loop {
        tokio::select! {
                line = read_bounded_line(&mut output, &mut output_line, MAX_TERMINAL_LINE) => {
                    match line {
                        Ok(0) => break,
                        Ok(_) => {
                            let value: Value = match serde_json::from_str(&output_line) {
                                Ok(value) => value,
                                Err(_) => {
                                    ended_with_error = true;
                                    let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "malformed_terminal", "terminal stream record is invalid")).await;
                                    break;
                                }
                            };
                            match value.get("type").and_then(Value::as_str) {
                                Some("terminal.frame") => {
                                    let seq = match value.get("seq").and_then(|v| v.as_u64()) {
                                        Some(seq) => seq,
                                        None => { ended_with_error = true; let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "malformed_terminal", "terminal frame sequence is invalid")).await; break; }
                                    };
                                    if let Some(previous) = previous_seq
                                        && seq != previous.saturating_add(1)
                                    {
                                        ended_with_error = true;
                                        let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "terminal_sequence_error", "terminal frame sequence is not consecutive")).await;
                                        break;
                                    }
                                    let encoding = value.get("encoding").and_then(Value::as_str);
                                    let width = value.get("width").and_then(Value::as_u64).and_then(|v| u16::try_from(v).ok());
                                    let height = value.get("height").and_then(Value::as_u64).and_then(|v| u16::try_from(v).ok());
                                    let full = value.get("full").and_then(Value::as_bool);
                                    let encoded = value.get("bytes").and_then(Value::as_str);
                                    let (Some(encoding), Some(width), Some(height), Some(full), Some(encoded)) = (encoding, width, height, full, encoded) else {
                                        ended_with_error = true;
                                        let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "malformed_terminal", "terminal frame fields are invalid")).await;
                                        break;
                                    };
                                    if previous_seq.is_none() && !full {
                                        ended_with_error = true;
                                        let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "terminal_baseline_required", "terminal stream must begin with a full frame")).await;
                                        break;
                                    }
                                    if encoding != "ansi" || base64::engine::general_purpose::STANDARD.decode(encoded).is_err() {
                                        ended_with_error = true;
                                        let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "malformed_terminal", "terminal frame encoding is invalid")).await;
                                        break;
                                    }
                                    previous_seq = Some(seq);
                                    if mode == TerminalMode::Control && !ownership_announced {
                                        ownership_announced = true;
                                        ownership = TerminalOwnershipState::Owned;
                                        let _ = sender.send(TerminalStreamMessage::Ownership { session_id: session_id.clone(), pane_id: pane_id.clone(), stream_id: stream_id.clone(), state: ownership, message: None }).await;
                                    }
                                    let _ = sender.send(TerminalStreamMessage::Frame { session_id: session_id.clone(), pane_id: pane_id.clone(), stream_id: stream_id.clone(), seq: seq.to_string(), encoding: encoding.to_owned(), width, height, full, bytes: encoded.to_owned() }).await;
                                }
                                Some("terminal.closed") => {
                                    closed = true;
                                    let reason = sanitize_terminal_text(
                                        value
                                            .get("reason")
                                            .and_then(Value::as_str)
                                            .unwrap_or("closed"),
                                    );
                                    let message = if mode == TerminalMode::Control
                                        && reason.eq_ignore_ascii_case("terminal attach taken over")
                                    {
                                        TerminalStreamMessage::Ownership {
                                            session_id: session_id.clone(),
                                            pane_id: pane_id.clone(),
                                            stream_id: stream_id.clone(),
                                            state: TerminalOwnershipState::Lost,
                                            message: Some(reason),
                                        }
                                    } else {
                                        TerminalStreamMessage::Closed {
                                            session_id: session_id.clone(),
                                            pane_id: pane_id.clone(),
                                            stream_id: stream_id.clone(),
                                            reason,
                                        }
                                    };
                                    let _ = sender.send(message).await;
                                    break;
                                }
                                Some(record_type) => {
                                    ended_with_error = true;
                                    let bounded_type: String = record_type.chars().take(64).collect();
                                    let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "unsupported_terminal_record", &format!("unsupported terminal record: {bounded_type}"))).await;
                                    break;
                                }
                                None => {
                                    ended_with_error = true;
                                    let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "malformed_terminal", "terminal stream record type is required")).await;
                                    break;
                                }
                            }
                        }
                        Err(_) => {
                            ended_with_error = true;
                            let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "bounded_output", "terminal frame line exceeds configured limit or is unterminated")).await;
                            break;
                        }
                    }
                }
                command = commands.recv() => {
                    let Some(command) = command else { break; };
                    if command.validate().is_err() { let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "invalid_terminal_command", "terminal command is invalid")).await; continue; }
                    if let TerminalCommand::Input { bytes: Some(encoded), .. } = &command
                        && base64::engine::general_purpose::STANDARD.decode(encoded).is_err()
                    {
                        let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "invalid_terminal_command", "terminal input bytes are not valid base64")).await;
                        continue;
                    }
                    let mutation = matches!(
                        &command,
                        TerminalCommand::Input { .. }
                            | TerminalCommand::Resize { .. }
                            | TerminalCommand::Scroll { .. }
                    );
                    if mutation && (mode == TerminalMode::Observe || !ownership_announced) {
                        let message = if mode == TerminalMode::Observe {
                            "terminal observe stream is read-only"
                        } else {
                            "terminal control is pending ownership"
                        };
                        let _ = sender
                            .send(terminal_error(
                                &session_id,
                                &pane_id,
                                &stream_id,
                                "terminal_command_rejected",
                                message,
                            ))
                            .await;
                        continue;
                    }
                    let release = matches!(command, TerminalCommand::Release);
                    match serde_json::to_string(&command) {
                        Ok(mut line) => {
                            line.push('\n');
                            if stdin.write_all(line.as_bytes()).await.is_err() { break; }
                            if release {
                                let _ = sender.send(TerminalStreamMessage::Ownership {
                                    session_id: session_id.clone(),
                                    pane_id: pane_id.clone(),
                                    stream_id: stream_id.clone(),
                                    state: TerminalOwnershipState::Released,
                                    message: None,
                                }).await;
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = sender.send(terminal_error(&session_id, &pane_id, &stream_id, "invalid_terminal_command", "terminal command could not be encoded")).await;
                        }
                    }
            }
        }
    }
    drop(stdin);
    let _ = child.kill().await;
    let _ = child.wait().await;
    let stderr = match stderr_task.await {
        Ok(Ok(stderr)) => stderr,
        Ok(Err(error)) => {
            ended_with_error = true;
            let _ = sender
                .send(terminal_error(
                    &session_id,
                    &pane_id,
                    &stream_id,
                    &error.code,
                    &error.message,
                ))
                .await;
            String::new()
        }
        Err(_) => String::new(),
    };
    if !closed && !ended_with_error {
        let lower = stderr.to_ascii_lowercase();
        if (lower.contains("already controlled"))
            || (lower.contains("control") && lower.contains("owned"))
            || (lower.contains("takeover") && lower.contains("required"))
        {
            let _ = sender
                .send(terminal_error(
                    &session_id,
                    &pane_id,
                    &stream_id,
                    "terminal_ownership_conflict",
                    "terminal ownership is held by another client",
                ))
                .await;
        } else {
            let _ = sender
                .send(TerminalStreamMessage::Disconnected {
                    session_id,
                    pane_id,
                    stream_id,
                    code: "terminal_attach_failed".into(),
                    message: "terminal stream disconnected".into(),
                })
                .await;
        }
    }
}

fn terminal_error(
    session_id: &str,
    pane_id: &str,
    stream_id: &str,
    code: &str,
    message: &str,
) -> TerminalStreamMessage {
    TerminalStreamMessage::Error {
        session_id: session_id.to_owned(),
        pane_id: pane_id.to_owned(),
        stream_id: stream_id.to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn parse_focus_result(
    result: Value,
    kind: FocusKind,
    target_id: &str,
) -> Result<bool, InspectionError> {
    let result = object(&result, "focus result")?;
    let result_type = required_string(result, "type", "focus result")?;
    let (expected_type, resource_key, target_key) = match kind {
        FocusKind::Space => ("workspace_info", "workspace", Some("workspace_id")),
        FocusKind::Tab => ("tab_info", "tab", Some("tab_id")),
        FocusKind::Pane => ("pane_info", "pane", Some("pane_id")),
        FocusKind::Agent => ("agent_info", "agent", Some("pane_id")),
    };
    if result_type != expected_type {
        return Err(InspectionError::new(
            "malformed_focus_response",
            "Herdr focus response has an unexpected result type",
        ));
    }
    let resource = result.get(resource_key).ok_or_else(|| {
        InspectionError::new(
            "malformed_focus_response",
            "Herdr focus response omitted its resource",
        )
    })?;
    let resource = object(resource, "focus result resource")?;
    if let Some(target_key) = target_key {
        let response_target = resource
            .get(target_key)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                InspectionError::new(
                    "malformed_focus_response",
                    "Herdr focus response omitted a valid resource identity",
                )
            })?;
        if response_target != target_id {
            return Err(InspectionError::new(
                "focus_target_mismatch",
                "Herdr focus response targets a different resource",
            ));
        }
    }
    if let Some(focused) = resource.get("focused") {
        if !focused.as_bool().unwrap_or(false) {
            return Err(InspectionError::new(
                "focus_not_confirmed",
                "Herdr focus response did not confirm focus",
            ));
        }
    } else {
        return Err(InspectionError::new(
            "malformed_focus_response",
            "Herdr focus response omitted focus state",
        ));
    }
    Ok(true)
}

fn split_direction(direction: PaneSplitDirection) -> &'static str {
    match direction {
        PaneSplitDirection::Right => "right",
        PaneSplitDirection::Down => "down",
    }
}

fn resize_direction(direction: PaneResizeDirection) -> &'static str {
    match direction {
        PaneResizeDirection::Left => "left",
        PaneResizeDirection::Right => "right",
        PaneResizeDirection::Up => "up",
        PaneResizeDirection::Down => "down",
    }
}

fn zoom_mode(mode: PaneZoomMode) -> &'static str {
    match mode {
        PaneZoomMode::Toggle => "toggle",
        PaneZoomMode::On => "on",
        PaneZoomMode::Off => "off",
    }
}

fn insert_optional(params: &mut serde_json::Map<String, Value>, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        params.insert(key.to_owned(), Value::String(value.clone()));
    }
}

fn insert_optional_number(
    params: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<f64>,
) {
    if let Some(value) = value {
        params.insert(key.to_owned(), json!(value));
    }
}

fn pane_move_destination(destination: &PaneMoveDestination) -> Value {
    match destination {
        PaneMoveDestination::ExistingTab {
            tab_id,
            direction,
            target_pane_id,
            ratio,
        } => {
            let mut params = serde_json::Map::from_iter([
                ("type".to_owned(), json!("tab")),
                ("tab_id".to_owned(), json!(tab_id)),
                ("split".to_owned(), json!(split_direction(*direction))),
            ]);
            insert_optional(&mut params, "target_pane_id", target_pane_id);
            insert_optional_number(&mut params, "ratio", *ratio);
            Value::Object(params)
        }
        PaneMoveDestination::NewTab { space_id, label } => {
            let mut params = serde_json::Map::from_iter([("type".to_owned(), json!("new_tab"))]);
            if let Some(space_id) = space_id {
                params.insert("workspace_id".to_owned(), json!(space_id));
            }
            insert_optional(&mut params, "label", label);
            Value::Object(params)
        }
        PaneMoveDestination::NewSpace { label, tab_label } => {
            let mut params =
                serde_json::Map::from_iter([("type".to_owned(), json!("new_workspace"))]);
            insert_optional(&mut params, "label", label);
            insert_optional(&mut params, "tab_label", tab_label);
            Value::Object(params)
        }
    }
}

fn mutation_call(request: &ResourceMutationRequest) -> (&'static str, Value) {
    match request {
        ResourceMutationRequest::SpaceCreate { cwd, label } => {
            let mut params = serde_json::Map::from_iter([("focus".to_owned(), json!(true))]);
            insert_optional(&mut params, "cwd", cwd);
            insert_optional(&mut params, "label", label);
            ("workspace.create", Value::Object(params))
        }
        ResourceMutationRequest::SpaceRename { space_id, label } => (
            "workspace.rename",
            json!({"workspace_id": space_id, "label": label}),
        ),
        ResourceMutationRequest::SpaceMoveBlock {
            space_ids,
            before_space_id,
        } => {
            let mut params =
                serde_json::Map::from_iter([("workspace_ids".to_owned(), json!(space_ids))]);
            if let Some(before_space_id) = before_space_id {
                params.insert("before_workspace_id".to_owned(), json!(before_space_id));
            }
            ("workspace.move_block", Value::Object(params))
        }
        ResourceMutationRequest::SpaceClose { space_id } => {
            ("workspace.close", json!({"workspace_id": space_id}))
        }
        ResourceMutationRequest::TabCreate { space_id, label } => {
            let mut params = serde_json::Map::from_iter([
                ("workspace_id".to_owned(), json!(space_id)),
                ("focus".to_owned(), json!(true)),
            ]);
            insert_optional(&mut params, "label", label);
            ("tab.create", Value::Object(params))
        }
        ResourceMutationRequest::TabRename { tab_id, label } => {
            ("tab.rename", json!({"tab_id": tab_id, "label": label}))
        }
        ResourceMutationRequest::TabMove {
            tab_id,
            insert_index,
        } => (
            "tab.move",
            json!({"tab_id": tab_id, "insert_index": insert_index}),
        ),
        ResourceMutationRequest::TabClose { tab_id } => ("tab.close", json!({"tab_id": tab_id})),
        ResourceMutationRequest::PaneSplit {
            pane_id,
            direction,
            ratio,
        } => {
            let mut params = serde_json::Map::from_iter([
                ("target_pane_id".to_owned(), json!(pane_id)),
                ("direction".to_owned(), json!(split_direction(*direction))),
                ("focus".to_owned(), json!(true)),
            ]);
            insert_optional_number(&mut params, "ratio", *ratio);
            ("pane.split", Value::Object(params))
        }
        ResourceMutationRequest::PaneResize {
            pane_id,
            direction,
            amount,
        } => (
            "pane.resize",
            json!({
                "pane_id": pane_id,
                "direction": resize_direction(*direction),
                "amount": amount
            }),
        ),
        ResourceMutationRequest::PaneRename { pane_id, label } => {
            ("pane.rename", json!({"pane_id": pane_id, "label": label}))
        }
        ResourceMutationRequest::PaneSwap {
            source_pane_id,
            target_pane_id,
        } => (
            "pane.swap",
            json!({
                "source_pane_id": source_pane_id,
                "target_pane_id": target_pane_id
            }),
        ),
        ResourceMutationRequest::PaneMove {
            pane_id,
            destination,
        } => (
            "pane.move",
            json!({
                "pane_id": pane_id,
                "destination": pane_move_destination(destination),
                "focus": true
            }),
        ),
        ResourceMutationRequest::PaneZoom { pane_id, mode } => (
            "pane.zoom",
            json!({"pane_id": pane_id, "mode": zoom_mode(*mode)}),
        ),
        ResourceMutationRequest::PaneClose { pane_id } => {
            ("pane.close", json!({"pane_id": pane_id}))
        }
    }
}

#[async_trait]
impl HerdrAdapter for HerdrCliAdapter {
    async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
        self.inspect_with_server_autostart(self.config.session())
            .await
    }

    async fn inspect_session(
        &self,
        session_id: &str,
    ) -> Result<HerdrCompatibility, InspectionError> {
        self.selected_session(session_id)?;
        let ping = self.socket_request(session_id, "ping", json!({})).await?;
        let ping = object(&ping, "ping result")?;
        if required_string(ping, "type", "ping result")? != "pong" {
            return Err(malformed("ping result.type must be pong"));
        }
        let ping_version = required_string(ping, "version", "ping result")?;
        let ping_protocol = required_u32(ping, "protocol", "ping result")?;
        let compatibility = self.inspect_inner(Some(session_id)).await?;
        match compatibility {
            HerdrCompatibility::Compatible { identity }
                if identity.version != ping_version || identity.protocol != ping_protocol =>
            {
                Ok(Self::incompatible(
                    Some(identity),
                    "session_identity_mismatch",
                    "session ping identity differs from executable discovery",
                ))
            }
            other => Ok(other),
        }
    }
    async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
        self.list_sessions().await
    }

    async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError> {
        self.read_snapshot(session_id).await
    }

    async fn focus(
        &self,
        session_id: &str,
        request: &FocusRequest,
    ) -> Result<FocusResponse, InspectionError> {
        self.selected_session(session_id)?;
        if !valid_pane_id(&request.target_id) {
            return Err(InspectionError::new(
                "invalid_target_id",
                "focus target contains unsupported characters",
            ));
        }
        let (method, params) = match request.kind {
            FocusKind::Space => (
                "workspace.focus",
                json!({"workspace_id": request.target_id}),
            ),
            FocusKind::Tab => ("tab.focus", json!({"tab_id": request.target_id})),
            FocusKind::Pane => ("pane.focus", json!({"pane_id": request.target_id})),
            FocusKind::Agent => ("agent.focus", json!({"target": request.target_id})),
        };
        let accepted = parse_focus_result(
            self.socket_request(session_id, method, params).await?,
            request.kind,
            &request.target_id,
        )?;
        Ok(FocusResponse {
            session_id: session_id.to_owned(),
            kind: request.kind,
            target_id: request.target_id.clone(),
            accepted,
        })
    }
    async fn mutate(
        &self,
        session_id: &str,
        request: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError> {
        self.mutate_resource(session_id, request).await
    }

    async fn subscribe_session(
        &self,
        session_id: &str,
        snapshot: &SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError> {
        self.subscribe_socket(session_id.to_owned(), snapshot.clone())
            .await
    }

    async fn open_terminal(
        &self,
        request: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError> {
        self.open_terminal_inner(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_title(value: Value) -> Option<String> {
        sanitized_title(value.as_object().unwrap(), "pane").unwrap()
    }

    #[test]
    fn pane_title_prefers_label_over_terminal_titles() {
        assert_eq!(
            parsed_title(json!({
                "label": "renamed pane",
                "terminal_title_stripped": "shell",
                "terminal_title": "terminal"
            })),
            Some("renamed pane".into())
        );
    }

    #[test]
    fn pane_title_falls_back_from_empty_label() {
        assert_eq!(
            parsed_title(json!({
                "label": "",
                "terminal_title_stripped": "shell",
                "terminal_title": "terminal"
            })),
            Some("shell".into())
        );
    }

    #[test]
    fn pane_title_sanitizes_label() {
        assert_eq!(
            parsed_title(json!({
                "label": "\u{1b}[31mrenamed\u{1b}[0m\u{0} pane",
                "terminal_title_stripped": "shell"
            })),
            Some("renamed pane".into())
        );
    }

    #[tokio::test]
    async fn bounded_line_reuse_clears_previous_record() {
        let input = "{\"type\":\"first\"}\n{\"type\":\"second\",\"text\":\"€\"}\n";
        let mut reader = BufReader::with_capacity(1, std::io::Cursor::new(input.as_bytes()));
        let mut line = String::from("stale record");
        let first = read_bounded_line(&mut reader, &mut line, MAX_TERMINAL_LINE)
            .await
            .unwrap();
        assert_eq!(first, "{\"type\":\"first\"}\n".len());
        assert_eq!(line, "{\"type\":\"first\"}\n");
        let second = read_bounded_line(&mut reader, &mut line, MAX_TERMINAL_LINE)
            .await
            .unwrap();
        assert_eq!(second, "{\"type\":\"second\",\"text\":\"€\"}\n".len());
        assert_eq!(line, "{\"type\":\"second\",\"text\":\"€\"}\n");
    }

    #[test]
    fn agent_focus_accepts_authoritative_agent_envelope() {
        let result = json!({
            "type": "agent_info",
            "agent": {"pane_id": "w1:p1", "focused": true}
        });

        assert!(parse_focus_result(result, FocusKind::Agent, "w1:p1").unwrap());
    }

    #[test]
    fn agent_focus_rejects_wrong_result_type() {
        let result = json!({
            "type": "pane_info",
            "pane": {"agent": "w1:p1", "focused": true}
        });

        let error = parse_focus_result(result, FocusKind::Agent, "w1:p1").unwrap_err();
        assert_eq!(error.code, "malformed_focus_response");
    }

    #[test]
    fn agent_focus_requires_owning_pane_identity() {
        let missing_identity = json!({
            "type": "agent_info",
            "agent": {"focused": true}
        });
        let mismatched_identity = json!({
            "type": "agent_info",
            "agent": {"pane_id": "w1:p2", "focused": true}
        });

        let error = parse_focus_result(missing_identity, FocusKind::Agent, "w1:p1").unwrap_err();
        assert_eq!(error.code, "malformed_focus_response");
        let error = parse_focus_result(mismatched_identity, FocusKind::Agent, "w1:p1").unwrap_err();
        assert_eq!(error.code, "focus_target_mismatch");
    }

    #[test]
    fn agent_focus_requires_positive_confirmation() {
        let result = json!({
            "type": "agent_info",
            "agent": {"pane_id": "w1:p1", "focused": false}
        });

        let error = parse_focus_result(result, FocusKind::Agent, "w1:p1").unwrap_err();
        assert_eq!(error.code, "focus_not_confirmed");
    }
    #[test]
    fn resource_mutations_map_to_documented_methods_and_params() {
        let cases = [
            (
                ResourceMutationRequest::SpaceCreate {
                    cwd: Some("/work".into()),
                    label: Some("Space".into()),
                },
                "workspace.create",
                json!({"cwd": "/work", "label": "Space", "focus": true}),
            ),
            (
                ResourceMutationRequest::SpaceRename {
                    space_id: "s1".into(),
                    label: "Space".into(),
                },
                "workspace.rename",
                json!({"workspace_id": "s1", "label": "Space"}),
            ),
            (
                ResourceMutationRequest::SpaceMoveBlock {
                    space_ids: vec!["s1".into(), "s2".into()],
                    before_space_id: Some("s3".into()),
                },
                "workspace.move_block",
                json!({"workspace_ids": ["s1", "s2"], "before_workspace_id": "s3"}),
            ),
            (
                ResourceMutationRequest::SpaceClose {
                    space_id: "s1".into(),
                },
                "workspace.close",
                json!({"workspace_id": "s1"}),
            ),
            (
                ResourceMutationRequest::TabCreate {
                    space_id: "s1".into(),
                    label: Some("Tab".into()),
                },
                "tab.create",
                json!({"workspace_id": "s1", "label": "Tab", "focus": true}),
            ),
            (
                ResourceMutationRequest::TabCreate {
                    space_id: "s1".into(),
                    label: None,
                },
                "tab.create",
                json!({"workspace_id": "s1", "focus": true}),
            ),
            (
                ResourceMutationRequest::TabRename {
                    tab_id: "t1".into(),
                    label: "Tab".into(),
                },
                "tab.rename",
                json!({"tab_id": "t1", "label": "Tab"}),
            ),
            (
                ResourceMutationRequest::TabMove {
                    tab_id: "t1".into(),
                    insert_index: 2,
                },
                "tab.move",
                json!({"tab_id": "t1", "insert_index": 2}),
            ),
            (
                ResourceMutationRequest::TabClose {
                    tab_id: "t1".into(),
                },
                "tab.close",
                json!({"tab_id": "t1"}),
            ),
            (
                ResourceMutationRequest::PaneSplit {
                    pane_id: "p1".into(),
                    direction: PaneSplitDirection::Right,
                    ratio: Some(0.4),
                },
                "pane.split",
                json!({"target_pane_id": "p1", "direction": "right", "ratio": 0.4, "focus": true}),
            ),
            (
                ResourceMutationRequest::PaneResize {
                    pane_id: "p1".into(),
                    direction: PaneResizeDirection::Up,
                    amount: 0.1,
                },
                "pane.resize",
                json!({"pane_id": "p1", "direction": "up", "amount": 0.1}),
            ),
            (
                ResourceMutationRequest::PaneRename {
                    pane_id: "p1".into(),
                    label: None,
                },
                "pane.rename",
                json!({"pane_id": "p1", "label": null}),
            ),
            (
                ResourceMutationRequest::PaneSwap {
                    source_pane_id: "p1".into(),
                    target_pane_id: "p2".into(),
                },
                "pane.swap",
                json!({"source_pane_id": "p1", "target_pane_id": "p2"}),
            ),
            (
                ResourceMutationRequest::PaneMove {
                    pane_id: "p1".into(),
                    destination: PaneMoveDestination::ExistingTab {
                        tab_id: "t2".into(),
                        direction: PaneSplitDirection::Down,
                        target_pane_id: Some("p2".into()),
                        ratio: Some(0.3),
                    },
                },
                "pane.move",
                json!({
                    "pane_id": "p1",
                    "destination": {
                        "type": "tab",
                        "tab_id": "t2",
                        "split": "down",
                        "target_pane_id": "p2",
                        "ratio": 0.3
                    },
                    "focus": true
                }),
            ),
            (
                ResourceMutationRequest::PaneZoom {
                    pane_id: "p1".into(),
                    mode: PaneZoomMode::Off,
                },
                "pane.zoom",
                json!({"pane_id": "p1", "mode": "off"}),
            ),
            (
                ResourceMutationRequest::PaneClose {
                    pane_id: "p1".into(),
                },
                "pane.close",
                json!({"pane_id": "p1"}),
            ),
        ];

        for (request, expected_method, expected_params) in cases {
            let (method, params) = mutation_call(&request);
            assert_eq!(method, expected_method);
            assert_eq!(params, expected_params);
        }
    }

    #[test]
    fn pane_move_maps_space_destinations_to_herdr_workspaces() {
        let new_tab = pane_move_destination(&PaneMoveDestination::NewTab {
            space_id: Some("s1".into()),
            label: None,
        });
        let new_space = pane_move_destination(&PaneMoveDestination::NewSpace {
            label: Some("Space".into()),
            tab_label: Some("Tab".into()),
        });

        assert_eq!(new_tab, json!({"type": "new_tab", "workspace_id": "s1"}));
        assert_eq!(
            new_space,
            json!({"type": "new_workspace", "label": "Space", "tab_label": "Tab"})
        );
    }

    #[test]
    fn worktree_list_projects_matching_repository_context() {
        let result = json!({
            "type": "worktree_list",
            "source": {
                "repo_key": "repo-opaque",
                "repo_name": "cockpit",
                "repo_root": "/work/cockpit",
                "source_checkout_path": "/work/cockpit"
            },
            "worktrees": [
                {
                    "path": "/work/cockpit",
                    "branch": "main",
                    "is_bare": false,
                    "is_detached": false,
                    "is_prunable": false,
                    "is_linked_worktree": false,
                    "label": "cockpit",
                    "open_workspace_id": "w18"
                },
                {
                    "path": "/work/cockpit-brave",
                    "branch": "worktree/brave-forest-7518",
                    "is_bare": false,
                    "is_detached": false,
                    "is_prunable": false,
                    "is_linked_worktree": true,
                    "label": "brave-forest-7518",
                    "open_workspace_id": "w1C"
                }
            ]
        });

        let main = parse_space_git_summary(&result, "w18").unwrap().unwrap();
        let linked = parse_space_git_summary(&result, "w1C").unwrap().unwrap();
        assert_eq!(main.repository_key, "repo-opaque");
        assert_eq!(linked.repository_key, main.repository_key);
        assert_eq!(main.repository, "cockpit");
        assert_eq!(main.branch.as_deref(), Some("main"));
        assert_eq!(main.checkout_path, "/work/cockpit");
        assert!(!main.is_linked_worktree);
        assert_eq!(linked.branch.as_deref(), Some("worktree/brave-forest-7518"));
        assert!(linked.is_linked_worktree);
    }

    #[test]
    fn worktree_list_ignores_other_workspaces_and_detached_branches() {
        let result = json!({
            "type": "worktree_list",
            "source": {"repo_key": "repo-opaque", "repo_name": "cockpit"},
            "worktrees": [{
                "path": "/work/detached",
                "branch": "stale-name",
                "is_detached": true,
                "is_linked_worktree": true,
                "open_workspace_id": "other"
            }]
        });

        assert_eq!(parse_space_git_summary(&result, "w18").unwrap(), None);
        let detached = parse_space_git_summary(&result, "other").unwrap().unwrap();
        assert_eq!(detached.branch, None);
    }

    #[test]
    fn malformed_worktree_list_is_rejected_for_fail_soft_enrichment() {
        assert!(parse_space_git_summary(&json!({"type": "other"}), "w18").is_err());
        assert!(
            parse_space_git_summary(
                &json!({
                    "type": "worktree_list",
                    "source": {"repo_key": "repo-opaque", "repo_name": "cockpit"},
                    "worktrees": [{"open_workspace_id": "w18", "path": 7}]
                }),
                "w18"
            )
            .is_err()
        );
    }

    #[test]
    fn required_schema_methods_cover_session_features() {
        assert_eq!(
            REQUIRED_METHODS,
            [
                "ping",
                "session.snapshot",
                "events.subscribe",
                "worktree.list",
                "pane.read",
                "workspace.create",
                "workspace.rename",
                "workspace.move_block",
                "workspace.close",
                "tab.create",
                "tab.rename",
                "tab.move",
                "tab.close",
                "pane.split",
                "pane.resize",
                "pane.rename",
                "pane.swap",
                "pane.move",
                "pane.zoom",
                "pane.close",
            ]
        );
    }
}
