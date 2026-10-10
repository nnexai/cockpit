use axum::{
    Json,
    extract::{
        Path, Query,
        rejection::{JsonRejection, QueryRejection},
    },
};
use cockpit_protocol::orchestration::{
    OperatorOrigin, OrchestrationMutationRequest, OrchestrationMutationResponse,
    OrchestrationSnapshot, OrchestrationSnapshotRequest, OrchestrationWaitRequest,
    OrchestrationWaitResponse,
};
use serde::Deserialize;

use crate::{
    OrchestrationRuntime,
    transport::{
        Transport,
        error::OperationError,
        http_input::{self, Reject},
    },
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotQuery {
    pub root_id: Option<String>,
}

pub fn snapshot_input() -> impl FnOnce(
    (Path<String>, Result<Query<SnapshotQuery>, QueryRejection>),
) -> Result<OrchestrationSnapshotRequest, OperationError> {
    |(path, query)| {
        let session_id =
            http_input::session(Reject("invalid_session_id", "Session ID is invalid"))((path,))?;
        let query: SnapshotQuery = http_input::mapped_query(Reject(
            "invalid_orchestration_request",
            "Invalid orchestration snapshot query",
        ))((query,))?;
        Ok(OrchestrationSnapshotRequest {
            session_id,
            root_id: query.root_id,
        })
    }
}

pub fn mutation_input() -> impl FnOnce(
    (
        Path<String>,
        Result<Json<OrchestrationMutationRequest>, JsonRejection>,
    ),
) -> Result<OrchestrationMutationRequest, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, OrchestrationMutationRequest) =
            http_input::session_body(
                Reject("invalid_session_id", "Session ID is invalid"),
                Reject(
                    "invalid_orchestration_request",
                    "Expected a bounded JSON orchestration request with valid fields",
                ),
            )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected(
                "session_mismatch",
                "Path and request session IDs differ",
            ));
        }
        Ok(request)
    }
}

pub async fn orchestration_snapshot(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: OrchestrationSnapshotRequest,
) -> Result<OrchestrationSnapshot, OperationError> {
    runtime.snapshot(&request).await.map_err(Into::into)
}

pub async fn orchestration_mutate(
    runtime: &OrchestrationRuntime,
    transport: Transport,
    request: OrchestrationMutationRequest,
) -> Result<OrchestrationMutationResponse, OperationError> {
    // The route is protected by an exact Origin guard; no wire field chooses the actor.
    // Only the main native GUI capability exposes this command. Caller JSON never selects an actor.
    let origin = match transport {
        Transport::Gateway => OperatorOrigin::Browser,
        Transport::Native => OperatorOrigin::Native,
    };
    runtime.mutate(origin, request).await.map_err(Into::into)
}

pub async fn orchestration_wait(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: OrchestrationWaitRequest,
) -> Result<OrchestrationWaitResponse, OperationError> {
    // No service mutex is held across the long poll.
    runtime.wait(&request).await.map_err(Into::into)
}
