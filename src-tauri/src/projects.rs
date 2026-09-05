use std::io::{self, Write};

use cockpit_core::CockpitService;
use cockpit_protocol::{
    projects::{
        ProjectConfiguration, RepositoryListResponse, WorkspaceOperation,
        WorkspaceOperationRequest, WorkspaceReconcileRequest, WorkspaceSetupPlan,
        WorkspaceSetupRequest,
    },
    v1::ErrorResponse,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tauri::State;

use super::{MAX_MUTATION_REQUEST_BYTES, inspection_error_response, stream_error};

struct RequestBudget(usize);

impl Write for RequestBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "request limit exceeded"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decode_request<T: DeserializeOwned>(value: Value) -> Result<T, ErrorResponse> {
    serde_json::to_writer(RequestBudget(MAX_MUTATION_REQUEST_BYTES), &value).map_err(|_| {
        stream_error(
            "invalid_project_request",
            "Expected a bounded JSON project request with valid fields",
        )
    })?;
    serde_json::from_value(value).map_err(|_| {
        stream_error(
            "invalid_project_request",
            "Expected a bounded JSON project request with valid fields",
        )
    })
}

#[tauri::command]
pub async fn cockpit_project_configuration(
    service: State<'_, CockpitService>,
) -> Result<ProjectConfiguration, ErrorResponse> {
    Ok(service
        .projects()
        .map_err(inspection_error_response)?
        .configuration())
}

#[tauri::command]
pub async fn cockpit_repositories(
    service: State<'_, CockpitService>,
) -> Result<RepositoryListResponse, ErrorResponse> {
    service
        .projects()
        .map_err(inspection_error_response)?
        .repositories()
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_plan(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceSetupPlan, ErrorResponse> {
    let request: WorkspaceSetupRequest = decode_request(request)?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .plan(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_start(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceOperation, ErrorResponse> {
    let request: WorkspaceOperationRequest = decode_request(request)?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .start(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_operation(
    session_id: String,
    operation_id: String,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceOperation, ErrorResponse> {
    service
        .projects()
        .map_err(inspection_error_response)?
        .get(&session_id, &operation_id)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_resume(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceOperation, ErrorResponse> {
    let request: WorkspaceOperationRequest = decode_request(request)?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .resume(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_cancel(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceOperation, ErrorResponse> {
    let request: WorkspaceOperationRequest = decode_request(request)?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .cancel(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_reconcile(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceOperation, ErrorResponse> {
    let request: WorkspaceReconcileRequest = decode_request(request)?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .reconcile(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}
