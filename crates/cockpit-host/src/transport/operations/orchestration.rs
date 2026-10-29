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
    LiaisonLookupRequest, LiaisonLookupResponse,
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

/// Installed command-line contract. This does not construct or contact a runtime.
/// Native GUI startup must not answer this as a GUI transport.
pub fn orchestration_protocol() -> serde_json::Value {
    serde_json::json!({
        "orchestration_protocol": 4,
        "store_schema": 4,
        "client_role": "host",
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotQuery {
    pub root_id: Option<String>,
}

type SnapshotInput = (Path<String>, Result<Query<SnapshotQuery>, QueryRejection>);

pub fn snapshot_input() -> impl FnOnce(SnapshotInput) -> Result<OrchestrationSnapshotRequest, OperationError> {
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

type MutationInput = (
    Path<String>,
    Result<Json<OrchestrationMutationRequest>, JsonRejection>,
);

pub fn mutation_input() -> impl FnOnce(MutationInput) -> Result<OrchestrationMutationRequest, OperationError> {
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

type LiaisonLookupInput = (Path<String>, Result<Json<LiaisonLookupRequest>, JsonRejection>);

pub fn liaison_lookup_input() -> impl FnOnce(LiaisonLookupInput) -> Result<LiaisonLookupRequest, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, LiaisonLookupRequest) =
            http_input::session_body(
                Reject("invalid_session_id", "Session ID is invalid"),
                Reject("invalid_orchestration_request", "Expected a liaison lookup request"),
            )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected(
                "session_mismatch", "Path and request session IDs differ",
            ));
        }
        Ok(request)
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSendRequest {
    pub session_id: String,
    pub reference: String,
    pub text: String,
}

type AgentSendInput = (Path<String>, Result<Json<AgentSendRequest>, JsonRejection>);

pub fn agent_send_input() -> impl FnOnce(AgentSendInput) -> Result<AgentSendRequest, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, AgentSendRequest) = http_input::session_body(
            Reject("invalid_session_id", "Session ID is invalid"),
            Reject("invalid_orchestration_request", "Expected an agent send request"),
        )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected("session_mismatch", "Path and request session IDs differ"));
        }
        Ok(request)
    }
}

pub async fn orchestration_agent_send(
    runtime: &OrchestrationRuntime, transport: Transport, request: AgentSendRequest,
) -> Result<serde_json::Value, OperationError> {
    let origin = match transport {
        Transport::Gateway => OperatorOrigin::Browser,
        Transport::Native => OperatorOrigin::Native,
    };
    runtime.send_agent(&request.session_id, &request.reference, request.text, origin).await.map_err(Into::into)
}


pub async fn orchestration_liaison_lookup(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: LiaisonLookupRequest,
) -> Result<LiaisonLookupResponse, OperationError> {
    runtime.liaison_lookup(&request).await.map_err(Into::into)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liaison_lookup_fences_path_session_and_unknown_fields() {
        let request = LiaisonLookupRequest {
            session_id: "session".into(),
            selected_root_id: None,
        };
        let matched = liaison_lookup_input()((Path("session".into()), Ok(Json(request.clone()))));
        assert!(matched.is_ok());
        let mismatched = liaison_lookup_input()((Path("other".into()), Ok(Json(request))));
        assert!(mismatched.is_err());
        assert!(serde_json::from_value::<LiaisonLookupRequest>(serde_json::json!({
            "session_id": "session", "selected_root_id": null,
            "project_key": "caller-asserted",
        })).is_err());
    }

}
