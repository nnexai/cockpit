use cockpit_core::CockpitService;
use cockpit_protocol::{
    quota::{QuotaStatusRequest, QuotaStatusResponse},
    v1::{
        AgentAttentionChangeRequest, AgentAttentionResponse,
        FocusRequest, FocusResponse, ResourceMutationRequest, ResourceMutationResponse,
        SessionListResponse, SessionSnapshotResponse, SpaceGitActionRequest,
        SpaceGitActionResponse, SpaceGitStatusResponse, StatusResponse,
        SpaceServiceStatusRequest, SpaceServiceStatusResponse,
        SpaceCpuStatusRequest, SpaceCpuStatusResponse,
        SpaceTagChangeRequest, SpaceTagsResponse,
    },
};

use crate::transport::{
    Transport,
    error::OperationError,
    guard::{valid_resource_id, valid_session_id},
    limits::{
        MUTATION_BYTES, SPACE_SERVICE_STATUS_MAX_PANES, SerializationLimitError,
        check_serialized_size,
    },
};

pub async fn status(
    service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<StatusResponse, OperationError> {
    Ok(service.status().await)
}

pub const UPDATE_SOURCE_ROOT: &str = env!("COCKPIT_SOURCE_ROOT");

#[derive(serde::Serialize)]
pub struct UpdateStatus {
    pub installed_commit: Option<String>,
    pub source_path: &'static str,
    pub commits_behind_main: Option<u64>,
}

pub async fn update_status(
    _service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<UpdateStatus, OperationError> {
    const ROOT: &str = UPDATE_SOURCE_ROOT;
    let build = option_env!("COCKPIT_BUILD_COMMIT");
    let git = |args: Vec<String>| async move {
        let run = |program: &str| {
            let mut command = tokio::process::Command::new(program);
            command.current_dir(ROOT).args(&args);
            command
        };
        match run("git").output().await {
            Ok(output) => Some(output),
            Err(_) => run("/usr/bin/git").output().await.ok(),
        }
    };
    let count = if cfg!(target_os = "linux") && std::path::Path::new(ROOT).join("scripts/restart-native.py").is_file() {
        if let Some(build) = build {
            git(vec!["rev-list".into(), "--count".into(), format!("{build}..main")]).await
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|count| count.trim().parse().ok())
        } else { None }
    } else { None };
    Ok(UpdateStatus { installed_commit: build.map(str::to_owned), source_path: ROOT, commits_behind_main: count })
}

pub async fn update_install(
    service: &CockpitService,
    _transport: Transport,
    (session_id, space_id): (String, String),
) -> Result<cockpit_protocol::quick_ask::QuickAskLocation, OperationError> {
    if !valid_session_id(&session_id) || !valid_resource_id(&space_id) {
        return Err(OperationError::rejected("invalid_update_target", "Invalid session or Space id"));
    }
    if !cfg!(target_os = "linux") {
        return Err(OperationError::rejected("update_unavailable", "Native update installation is available only on Linux"));
    }
    let root = std::path::Path::new(UPDATE_SOURCE_ROOT);
    if !root.is_absolute() || !root.is_dir() || !root.join("scripts/restart-native.py").is_file() {
        return Err(OperationError::rejected("update_unavailable", "The source checkout is unavailable"));
    }
    let bytes = tokio::fs::read(root.join(".herdr/actions.json")).await
        .map_err(|_| OperationError::rejected("update_unavailable", "The update action is unavailable"))?;
    let config: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| OperationError::rejected("update_unavailable", "The update action configuration is invalid"))?;
    let actions = config["actions"].as_array().ok_or_else(|| OperationError::rejected("update_unavailable", "The update action configuration is invalid"))?;
    let mut matches = actions.iter().filter(|action| action["id"] == "cockpit-install-restart");
    let action = matches.next().ok_or_else(|| OperationError::rejected("update_unavailable", "The update action is unavailable"))?;
    if matches.next().is_some() || action["mode"] != "tab" || action["cwd"] != "worktree" {
        return Err(OperationError::rejected("update_unavailable", "The update action configuration is invalid"));
    }
    let command: Vec<String> = serde_json::from_value(action["command"].clone())
        .map_err(|_| OperationError::rejected("update_unavailable", "The update action command is invalid"))?;
    if command.is_empty() || command.iter().any(|arg| arg.contains('\0')) || command[0].is_empty() {
        return Err(OperationError::rejected("update_unavailable", "The update action command is invalid"));
    }
    let label = action["tabLabel"].as_str().filter(|label| !label.is_empty())
        .ok_or_else(|| OperationError::rejected("update_unavailable", "The update action label is invalid"))?;
    service.quick_ask()?.install_update(&session_id, &space_id, UPDATE_SOURCE_ROOT, &command, label).await.map_err(Into::into)
}

pub async fn quota_status(
    service: &CockpitService,
    _transport: Transport,
    request: QuotaStatusRequest,
) -> Result<QuotaStatusResponse, OperationError> {
    Ok(service.quota()?.status(request).await)
}

pub async fn sessions(
    service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<SessionListResponse, OperationError> {
    service.sessions().await.map_err(Into::into)
}

pub async fn session_snapshot(
    service: &CockpitService,
    _transport: Transport,
    session_id: String,
) -> Result<SessionSnapshotResponse, OperationError> {
    service
        .session_snapshot(&session_id)
        .await
        .map_err(Into::into)
}

pub async fn space_git_status(
    service: &CockpitService,
    _transport: Transport,
    session_id: String,
) -> Result<SpaceGitStatusResponse, OperationError> {
    service
        .space_git_status(&session_id)
        .await
        .map_err(Into::into)
}

pub async fn space_service_status(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, SpaceServiceStatusRequest),
) -> Result<SpaceServiceStatusResponse, OperationError> {
    if !valid_session_id(&session_id) {
        return Err(OperationError::rejected(
            "invalid_session_id",
            "Invalid session id",
        ));
    }
    if request.server_instance.is_empty()
        || request.server_instance.len() > 256
        || request.server_instance.chars().any(char::is_control)
        || request.pane_ids.len() > SPACE_SERVICE_STATUS_MAX_PANES
        || request.pane_ids.iter().any(|id| !valid_resource_id(id))
    {
        return Err(OperationError::rejected(
            "invalid_space_service_status_request",
            "Expected a bounded service-status pane set with a valid server identity",
        ));
    }
    service
        .space_service_status(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn space_cpu_status(
    service: &CockpitService, _transport: Transport,
    (session_id, request): (String, SpaceCpuStatusRequest),
) -> Result<SpaceCpuStatusResponse, OperationError> {
    if !valid_session_id(&session_id) {
        return Err(OperationError::rejected("invalid_session_id", "Invalid session id"));
    }
    let mut ids = std::collections::HashSet::new();
    if request.server_instance.is_empty() || request.server_instance.len() > 256
        || request.server_instance.chars().any(char::is_control)
        || request.panes.len() > SPACE_SERVICE_STATUS_MAX_PANES
        || request.panes.iter().any(|p| !valid_resource_id(&p.pane_id) || !valid_resource_id(&p.space_id) || !valid_resource_id(&p.terminal_id) || !ids.insert(&p.pane_id))
    {
        return Err(OperationError::rejected("invalid_space_cpu_status_request", "Expected bounded CPU sampling pane set"));
    }
    service.space_cpu_status(&session_id, &request).await.map_err(Into::into)
}

pub async fn space_tags(
    service: &CockpitService,
    _transport: Transport,
    session_id: String,
) -> Result<SpaceTagsResponse, OperationError> {
    service.space_tags()?.list(&session_id).await.map_err(Into::into)
}

pub async fn space_tag_change(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, SpaceTagChangeRequest),
) -> Result<SpaceTagsResponse, OperationError> {
    service.space_tags()?.change(&session_id, &request).await.map_err(Into::into)
}

pub async fn agent_attention(
    service: &CockpitService,
    _transport: Transport,
    session_id: String,
) -> Result<AgentAttentionResponse, OperationError> {
    service.agent_attention()?.list(&session_id).await.map_err(Into::into)
}

pub async fn agent_attention_change(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, AgentAttentionChangeRequest),
) -> Result<AgentAttentionResponse, OperationError> {
    service.agent_attention()?.change(&session_id, &request).await.map_err(Into::into)
}

pub async fn space_git_action(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, SpaceGitActionRequest),
) -> Result<SpaceGitActionResponse, OperationError> {
    service
        .space_git_action(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn focus(
    service: &CockpitService,
    transport: Transport,
    (session_id, request): (String, FocusRequest),
) -> Result<FocusResponse, OperationError> {
    if transport == Transport::Gateway
        && (!valid_session_id(&session_id) || !valid_resource_id(&request.target_id))
    {
        return Err(OperationError::rejected(
            "invalid_focus_target",
            "Invalid focus target",
        ));
    }
    service
        .focus(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn mutate(
    service: &CockpitService,
    transport: Transport,
    (session_id, request): (String, ResourceMutationRequest),
) -> Result<ResourceMutationResponse, OperationError> {
    match transport {
        Transport::Gateway => {
            if !valid_session_id(&session_id) {
                return Err(OperationError::rejected(
                    "invalid_session_id",
                    "Invalid session id",
                ));
            }
        }
        Transport::Native => {
            check_serialized_size(&request, MUTATION_BYTES).map_err(|error| match error {
                SerializationLimitError::Invalid(_) => {
                    OperationError::rejected("invalid_mutation_request", "Invalid mutation request")
                }
                SerializationLimitError::TooLarge => OperationError::rejected(
                    "mutation_request_too_large",
                    "Mutation request exceeds the 64 KiB limit",
                ),
            })?;
        }
    }
    service
        .mutate(&session_id, &request)
        .await
        .map_err(Into::into)
}
