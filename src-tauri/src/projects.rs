use cockpit_core::CockpitService;
use cockpit_protocol::{
    project_teardown::{
        WorkspaceTeardownExecuteRequest, WorkspaceTeardownPreview, WorkspaceTeardownPreviewRequest,
        WorkspaceTeardownRecoveryList, WorkspaceTeardownResult,
    },
    projects::{
        ProjectConfiguration, RepositoryListResponse, WorkspaceOperation,
        WorkspaceOperationRequest, WorkspaceReconcileRequest, WorkspaceSetupPlan,
        WorkspaceSetupRequest,
    },
    v1::ErrorResponse,
};
use serde_json::Value;
use tauri::State;

use super::{inspection_error_response, requests::decode_request};

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
    let request: WorkspaceSetupRequest = decode_request(request, "project")?;
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
    let request: WorkspaceOperationRequest = decode_request(request, "project")?;
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
    let request: WorkspaceOperationRequest = decode_request(request, "project")?;
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
    let request: WorkspaceOperationRequest = decode_request(request, "project")?;
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
    let request: WorkspaceReconcileRequest = decode_request(request, "project")?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .reconcile(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_teardown_preview(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceTeardownPreview, ErrorResponse> {
    let request: WorkspaceTeardownPreviewRequest = decode_request(request, "project teardown")?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .teardown_preview(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_teardown_execute(
    session_id: String,
    request: Value,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceTeardownResult, ErrorResponse> {
    let request: WorkspaceTeardownExecuteRequest = decode_request(request, "project teardown")?;
    service
        .projects()
        .map_err(inspection_error_response)?
        .teardown_execute(&session_id, &request)
        .await
        .map_err(inspection_error_response)
}

#[tauri::command]
pub async fn cockpit_workspace_teardown_recoveries(
    session_id: String,
    service: State<'_, CockpitService>,
) -> Result<WorkspaceTeardownRecoveryList, ErrorResponse> {
    service
        .projects()
        .map_err(inspection_error_response)?
        .teardown_recoveries(&session_id)
        .map_err(inspection_error_response)
}
