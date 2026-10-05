use super::{inspection_error_response, requests::decode_request};
use cockpit_host::OrchestrationRuntime;
use cockpit_protocol::{orchestration::*, v1::ErrorResponse};
use serde_json::Value;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn orchestration_snapshot(
    request: Value,
    runtime: State<'_, Arc<OrchestrationRuntime>>,
) -> Result<OrchestrationSnapshot, ErrorResponse> {
    let request: OrchestrationSnapshotRequest = decode_request(request, "orchestration")?;
    runtime
        .snapshot(&request)
        .await
        .map_err(inspection_error_response)
}
#[tauri::command]
pub async fn orchestration_mutate(
    request: Value,
    runtime: State<'_, Arc<OrchestrationRuntime>>,
) -> Result<OrchestrationMutationResponse, ErrorResponse> {
    let request: OrchestrationMutationRequest = decode_request(request, "orchestration")?;
    // Only the main native GUI capability exposes this command. Caller JSON never selects an actor.
    runtime
        .mutate(OperatorOrigin::Native, request)
        .await
        .map_err(inspection_error_response)
}
#[tauri::command]
pub async fn orchestration_wait(
    request: Value,
    runtime: State<'_, Arc<OrchestrationRuntime>>,
) -> Result<OrchestrationWaitResponse, ErrorResponse> {
    let request: OrchestrationWaitRequest = decode_request(request, "orchestration")?;
    runtime
        .wait(&request)
        .await
        .map_err(inspection_error_response)
}
