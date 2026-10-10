use cockpit_core::CockpitService;
use cockpit_protocol::{
    project_defaults::{WorkspaceDefaults, WorkspaceDefaultsRequest},
    project_teardown::{
        WorkspaceTeardownExecuteRequest, WorkspaceTeardownPreview, WorkspaceTeardownPreviewRequest,
        WorkspaceTeardownRecoveryList, WorkspaceTeardownResult,
    },
    projects::{
        ProjectConfiguration, RepositoryListResponse, WorkspaceOperation,
        WorkspaceOperationRequest, WorkspaceReconcileRequest, WorkspaceSetupPlan,
        WorkspaceSetupRequest,
    },
};

use crate::transport::{Transport, error::OperationError};

pub async fn project_configuration(
    service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<ProjectConfiguration, OperationError> {
    Ok(service.projects()?.configuration())
}

pub async fn repositories(
    service: &CockpitService,
    _transport: Transport,
    (): (),
) -> Result<RepositoryListResponse, OperationError> {
    service.projects()?.repositories().await.map_err(Into::into)
}

pub async fn workspace_defaults(
    service: &CockpitService,
    _transport: Transport,
    request: WorkspaceDefaultsRequest,
) -> Result<WorkspaceDefaults, OperationError> {
    service
        .projects()?
        .resolve_defaults(&request)
        .await
        .map_err(Into::into)
}

pub async fn workspace_plan(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, WorkspaceSetupRequest),
) -> Result<WorkspaceSetupPlan, OperationError> {
    service
        .projects()?
        .plan(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn workspace_start(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, WorkspaceOperationRequest),
) -> Result<WorkspaceOperation, OperationError> {
    let projects = service.projects()?;
    let library = service.library()?.clone();
    projects
        .start(&session_id, &request, library)
        .await
        .map_err(Into::into)
}

pub async fn workspace_operation(
    service: &CockpitService,
    _transport: Transport,
    (session_id, operation_id): (String, String),
) -> Result<WorkspaceOperation, OperationError> {
    service
        .projects()?
        .get(&session_id, &operation_id)
        .await
        .map_err(Into::into)
}

pub async fn workspace_resume(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, WorkspaceOperationRequest),
) -> Result<WorkspaceOperation, OperationError> {
    let projects = service.projects()?;
    let library = service.library()?.clone();
    projects
        .resume(&session_id, &request, library)
        .await
        .map_err(Into::into)
}

pub async fn workspace_cancel(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, WorkspaceOperationRequest),
) -> Result<WorkspaceOperation, OperationError> {
    service
        .projects()?
        .cancel(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn workspace_reconcile(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, WorkspaceReconcileRequest),
) -> Result<WorkspaceOperation, OperationError> {
    service
        .projects()?
        .reconcile(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn workspace_teardown_preview(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, WorkspaceTeardownPreviewRequest),
) -> Result<WorkspaceTeardownPreview, OperationError> {
    service
        .projects()?
        .teardown_preview(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn workspace_teardown_execute(
    service: &CockpitService,
    _transport: Transport,
    (session_id, request): (String, WorkspaceTeardownExecuteRequest),
) -> Result<WorkspaceTeardownResult, OperationError> {
    service
        .projects()?
        .teardown_execute(&session_id, &request)
        .await
        .map_err(Into::into)
}

pub async fn workspace_teardown_recoveries(
    service: &CockpitService,
    _transport: Transport,
    session_id: String,
) -> Result<WorkspaceTeardownRecoveryList, OperationError> {
    service
        .projects()?
        .teardown_recoveries(&session_id)
        .map_err(Into::into)
}
